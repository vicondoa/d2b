//! The EndpointBinding resource driver: the v3 `ResourceDriver` conversion of
//! one exact endpoint delivered to one admitted consumer.
//!
//! The driver owns the ROW side of the family and nothing else. The source
//! side - which consumer an `Endpoint` admits, which attachment kind reaches
//! it, which facets realize it - is [`d2b_provider_endpoint`]'s own binding
//! knowledge, and this driver reuses it rather than restating it: the
//! endpoint's own consumer policy, the operation one attachment kind
//! performs, the attachment ceiling, the required realization facets, and
//! the POSIX bits one admitted right needs all come from the Endpoint
//! provider, so a binding row can never reach an endpoint through a shape its
//! owner did not declare.
//!
//! # The row names one exact endpoint, and this driver proves it
//!
//! A committed `EndpointBinding` row carries the closed
//! [`EndpointBindingSpec`]: the bound `Endpoint`, the consumer that receives
//! it, the attachment kind, and the stable consumer slot the delivery
//! occupies. It has no path field and no purpose of its own - the purpose is
//! the endpoint row's - so nothing in the desired state can be rewritten into
//! a different socket. The row contract's own `new()` has already refused a
//! source that is not an `Endpoint` and a consumer this binding kind does not
//! admit. What the row must still PROVE is that its endpoint selector names
//! exactly one endpoint in this row's own Zone:
//! [`exact_endpoint_selector`] refuses a wildcard, an alternation, a
//! cross-Zone, or wrong-type selector before the typed contract is
//! ever consulted, and the named `Endpoint` row is then read through the
//! manager, owner-fenced against this row, and decoded as the closed
//! `EndpointSpec`.
//!
//! # Readiness is the verified descriptor, never an assumed one
//!
//! Reconcile never assumes the endpoint is reachable. It asks the effect port
//! ([`EndpointBindingDriverEffects`]) what the kernel actually applies for this
//! consumer - the pinned `(dev, ino)` the endpoint owner resolved privately,
//! the named ACL entry already ANDed with the mask, the AND across every
//! ancestor's effective traverse bit, and whether the endpoint is accepting -
//! and it refuses before any delivery when the effective access is short of
//! the admitted right, an ancestor no longer applies a traverse bit, the
//! containing directory is listable (the authority the family exists to
//! remove), or an `attach` names an endpoint that is not accepting. A pass
//! that cannot prove the descriptor delivers nothing and publishes the stable
//! not-ready code instead.
//!
//! # The row is the source's decision, and it is checked
//!
//! A committed row carries the source's own [`BindingSourceDecision`] beside
//! its references, and the driver realizes the row only under that authority:
//! the right the row's attachment kind claims must be one the source
//! admitted, and every facet that delivery depends on must be one the source
//! declared. Both checks run in the row seam, so no verb can act on a
//! decision it never verified - not validate, not reconcile, not the
//! teardown. This is a different question from the one
//! [`endpoint_binding_support`] answers: that one asks what this
//! implementation can realize at all, and both are required.
//!
//! # A replaced inode invalidates a cached delivery
//!
//! The pinned identity is compared on every pass against the freshly observed
//! one. An endpoint whose inode was replaced (or whose ACL mask a later
//! `chmod` nullified) stops reporting a delivery: the relationship re-delivers
//! against the NEW exact endpoint or reports degraded, and never keeps handing
//! a consumer the endpoint that was there before. Restart is the same rule
//! with no cached identity to compare: recover VERIFIES the exact endpoint and
//! adopts it, and never re-delivers.
//!
//! # Teardown is detach-then-release
//!
//! The durable deleting mark does not cut a consumer off from an endpoint it
//! still holds. `pre_drain` fences new use, `delete` observes whether the
//! consumer is still attached, and an attached consumer blocks the teardown
//! retryably with the delivery left in place; only a detached consumer's
//! delivery is released, idempotently, so a requeued pass converges.
//!
//! Conversion mapping:
//! - `describe` -> [`binding_descriptor`] registration under `EndpointBinding`.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the exact-endpoint
//!   selector, the named `Endpoint` row, and the owner fence.
//! - `observe` -> [`ResourceDriver::recover`]: verify and re-adopt.
//! - `binding_children`/readiness -> [`ResourceDriver::reconcile`].
//! - `finalize_binding` -> [`ResourceDriver::pre_drain`] then
//!   [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only) plus the fenced
//!   projection `ctx.set_status_projection` publishes.
//!
//! The generic child-first [`ResourceDriver::finalize`] step is deliberately
//! left at its default: this family owns no children
//! ([`ENDPOINT_BINDING_CREATIONS`] is empty), so there is nothing for it to
//! drain ahead of the row's own teardown.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec;
use d2b_contracts_resource::v3::{
    BindingRealizationFacet, CanonicalJsonObject, EndpointAttachmentKind, RequestedRights,
    ResourceGeneration,
    ResourceRef, ResourceSpec, ResourceUid, ZoneId, ZoneRevision, execution_policy::BoundedToken,
    resource_status::StatusCode,
};
use d2b_provider_endpoint::{
    DeliveryForm, EndpointAccessObservation, EndpointBindingError, EndpointSocketIdentity,
    declared_delivery_form, endpoint_binding_support, required_right_bits,
};
use d2b_provider_endpoint::endpoint::{EndpointConsumerPolicy, EndpointSpec};
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

use crate::effects_service::BINDING_EFFECTS_SERVICE;

/// The one resource type this factory serves.
pub const ENDPOINT_BINDING_TYPE_NAME: &str = "EndpointBinding";

/// The ResourceType of the exact endpoint one relationship delivers.
pub const ENDPOINT_TYPE_NAME: &str = "Endpoint";

/// The serving Provider reference one committed `EndpointBinding` row may
/// name.
///
/// The row's `providerRef` names the Provider that owns the relationship's
/// realization, and the Endpoint family owns it: the endpoint's own locator
/// and the descriptor the consumer receives are resolved by the effect
/// adapter the Endpoint Provider installs. A row naming a Provider outside
/// this family is refused; a row naming no Provider is served by the
/// declaring Provider, which is this type's own driver.
pub const BINDING_PROVIDER_REF: &str = "Provider/endpoint";

/// The children this driver mints: none.
///
/// An `EndpointBinding` delivers an `Endpoint` that is already committed and
/// that the row is owned by. Minting an `Endpoint` child here would be a
/// second, unauthorized endpoint - exactly the neighbouring endpoint the
/// family exists to keep a consumer away from - so the declaration licenses
/// no child creation and the driver never calls `ensure_child`. The named
/// endpoint is a DEPENDENCY the driver reads through the manager (see
/// [`ENDPOINT_BINDING_READS`]), never a child it owns.
pub const ENDPOINT_BINDING_CREATIONS: &[ChildCreation] = &[];

/// The resource types the binding driver reads while reconciling.
///
/// The driver resolves the exact `Endpoint` row through the manager: for its
/// own declared policy (the consumer allowlist, the operation allowlist, and
/// the attachment ceiling that decide what this relationship may do) and for
/// the owner fence that binds the row to the endpoint it delivers.
pub const ENDPOINT_BINDING_READS: &[WellKnownType] = &[WellKnownType::ENDPOINT];

/// The execution domains the EndpointBinding type can be reconciled in.
///
/// Derived from the placement contract: `EndpointBinding` names no placement
/// anchor, so a binding row never carries the canonical `spec.executionRef`
/// and the plane reconciles it on its containing Zone's Host. The consumer
/// the endpoint is delivered to runs where that consumer runs; the delivery
/// itself is realized by the Endpoint family's effect adapter.
const ENDPOINT_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// Preserved re-check cadence while the exact endpoint is not yet proven:
/// a short repair interval, so a descriptor that is still coming up
/// converges without depending on a watch delivery.
const ENDPOINT_BINDING_RESYNC: Duration = Duration::from_secs(2);

/// The stable not-ready code published while the exact endpoint has not been
/// proven reachable for this consumer.
///
/// The code is field-free: it names the class of shortfall, never the socket,
/// the consumer, or the host bytes it was protecting.
pub const ENDPOINT_NOT_READY: &str = "endpoint-not-ready";

/// The stable not-ready code published while the delivery this row handed
/// out no longer names the endpoint the owner resolved.
pub const ENDPOINT_IDENTITY_REPLACED: &str = "endpoint-identity-replaced";

/// The stable not-ready code published while the endpoint's containing
/// directory still admits enumeration by the consumer principal.
pub const ENDPOINT_DIRECTORY_LISTABLE: &str = "endpoint-directory-listable";

// ---------------------------------------------------------------------------
// The exact-endpoint selector
// ---------------------------------------------------------------------------

/// Why one committed `sourceRef` does not name exactly one endpoint.
///
/// Every variant is a refusal. There is no variant meaning "resolve it as
/// best you can" and none meaning "a neighbouring endpoint is close enough",
/// so a selector that could match more than the one committed endpoint has no
/// success path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointSelectorRejection {
    /// The base spec carries no `sourceRef` at all.
    Missing,
    /// The `sourceRef` is present but is not a string.
    NotAString,
    /// The selector carries a wildcard, glob, or alternation character, so it
    /// could name more than the one committed endpoint.
    Wildcard,
    /// The selector is zone-qualified or names more than one component, so it
    /// reaches across the boundary this row is reconciled inside.
    CrossZone,
    /// The selector names a ResourceType this binding kind does not deliver.
    WrongType,
}

impl EndpointSelectorRejection {
    /// The stable reason this refusal reports, for the failure comparison
    /// and the driver's in-memory status.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "endpoint-selector-missing",
            Self::NotAString => "endpoint-selector-not-a-string",
            Self::Wildcard => "endpoint-selector-wildcard",
            Self::CrossZone => "endpoint-selector-cross-zone",
            Self::WrongType => "endpoint-selector-wrong-type",
        }
    }
}

impl core::fmt::Display for EndpointSelectorRejection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for EndpointSelectorRejection {}

/// The selector characters that could name more than one endpoint.
///
/// A `*` or `?` globs, a comma or pipe alternates, and a `%` is the
/// wildcard placeholder an encoded selector uses. None of them can appear in
/// a canonical `ResourceName` (`^[a-z][a-z0-9-]{0,62}$`), so their presence is
/// an attempt to widen the grant, not a spelling.
const SELECTOR_WILDCARDS: [char; 5] = ['*', '?', ',', '|', '%'];

/// The ONE place a committed `EndpointBinding` row's field set is read.
///
/// Everything else in this crate reaches a row field through an accessor of
/// the row contract [`EndpointBindingSpec`], so the shape a committed row is
/// written in is named here and nowhere else. Repointing the crate at a
/// different row contract is therefore a single edit, and nothing else in the
/// crate has to move with it.
pub fn committed_source_selector(base: &serde_json::Value) -> Result<&str, EndpointSelectorRejection> {
    let Some(selector) = base.get(ENDPOINT_REF_FIELD) else {
        return Err(EndpointSelectorRejection::Missing);
    };
    let Some(selector) = selector.as_str() else {
        return Err(EndpointSelectorRejection::NotAString);
    };
    exact_endpoint_selector(selector)?;
    Ok(selector)
}

/// The base-spec field that carries the exact endpoint a row names, and the
/// compared field its refusals report it under.
///
/// These two constants are the only place in the crate that names a
/// committed row's wire field, and they sit with the one function that
/// reads it. They agree with `EndpointBindingSpec`'s own
/// `#[serde(rename_all = "camelCase")]` spelling of `endpoint_ref`.
const ENDPOINT_REF_FIELD: &str = "endpointRef";
const ENDPOINT_REF_COMPARISON_FIELD: &str = "spec.endpointRef";

/// Apply the exact-endpoint rule to one selector, and return the selector
/// when it names exactly one endpoint in the Zone the row lives in.
///
/// The rule is that a relationship names ONE endpoint:
///
/// - it must carry no wildcard, glob, or alternation character, because a
///   selector that could match more than the committed endpoint is exactly
///   the grant this family refuses to widen;
/// - it must carry no zone qualifier and no second separator, because a
///   binding is a same-Zone relationship and a cross-Zone target is never
///   served from this row;
/// - its type component must be exactly `Endpoint`, the source this binding
///   kind delivers.
///
/// The rule takes the selector STRING and knows nothing about where a row
/// carries it, so it holds whichever field name the row contract uses. The
/// check runs on the string rather than on a decoded reference on purpose:
/// the canonical `ResourceName` grammar would reject a wildcard anyway, but
/// it would reject it as an opaque decode failure, and this rule is the
/// admission decision that names which shape was attempted.
pub fn exact_endpoint_selector(selector: &str) -> Result<(), EndpointSelectorRejection> {
    if selector.contains(SELECTOR_WILDCARDS) {
        return Err(EndpointSelectorRejection::Wildcard);
    }
    if selector.contains(':') || selector.matches('/').count() != 1 {
        return Err(EndpointSelectorRejection::CrossZone);
    }
    let (resource_type, _name) = selector
        .split_once('/')
        .ok_or(EndpointSelectorRejection::CrossZone)?;
    if resource_type != ENDPOINT_TYPE_NAME {
        return Err(EndpointSelectorRejection::WrongType);
    }
    Ok(())
}

/// Parse one committed `EndpointBinding` row into the contract's own row
/// type.
///
/// This is the one place a row is read. The row contract is
/// [`EndpointBindingSpec`] from `d2b-contracts-resource`, and its own
/// `new()` - reached through its strict wire decoder - has already refused
/// a source that is not an `Endpoint` and a consumer this binding kind does
/// not admit. The exact-endpoint rule runs first, on the raw base, so a
/// wildcard, an alternation, or a cross-Zone target is refused as the shape
/// it is rather than as an opaque decode failure.
///
/// The driver never reconstructs the canonical source-side request from
/// this row: every value it needs is an accessor of the row or of the
/// resolved `EndpointSpec`. The delivery's purpose is the ENDPOINT row's,
/// which is where `EndpointProvenance` derives it from too, so a binding
/// row and a consumer request cannot drift apart through a second copy of a
/// field.
fn committed_row(
    base: &serde_json::Value,
    op: DriverOp,
) -> Result<EndpointBindingSpec, EndpointBindingDriverError> {
    committed_source_selector(base).map_err(|rejection| {
        EndpointBindingDriverError::new(EndpointBindingDriverErrorKind::SpecInvalid, op)
            .with_detail(FailureDetail::at("spec/source").comparison(
                FailureComparison::new(
                    ENDPOINT_REF_COMPARISON_FIELD,
                    "one exact Endpoint/<name> in this Zone",
                    rejection.code(),
                ),
            ))
    })?;
    let row = serde_json::from_value::<EndpointBindingSpec>(base.clone()).map_err(|_| {
        EndpointBindingDriverError::new(EndpointBindingDriverErrorKind::SpecInvalid, op)
    })?;
    admitted_source_decision(&row, op)?;
    Ok(row)
}

/// The row's committed source decision, which is the authority this driver
/// realizes a delivery under.
///
/// Two questions, and they are not the same one:
///
/// - [`admitted_rights`](d2b_contracts_resource::v3::BindingSourceDecision::admitted_rights) answers "did the SOURCE
///   admit this consumer for this right". It is the authority the row
///   carries, so a row whose right the source withheld must never be
///   delivered, however capable this implementation is.
/// - [`realized_facets`](d2b_contracts_resource::v3::BindingSourceDecision::realized_facets) answers "did the source
///   DECLARE the facet this delivery is realized through". The requirement
///   is the attachment kind's own `required_facets`: a descriptor delivery
///   needs the endpoint descriptor alone, and an `attach` additionally needs
///   the private pathname.
///
/// A third question is answered elsewhere and deliberately not here:
/// [`endpoint_binding_support`] is what this IMPLEMENTATION can realize at
/// all, and [`endpoint_policy_refusal`] checks it against the endpoint's own
/// declaration. "Support" and "admitted" are easy to confuse on a later read,
/// and confusing them is exactly how a row gets realized under authority the
/// source withheld, so both checks stand.
fn admitted_source_decision(
    row: &EndpointBindingSpec,
    op: DriverOp,
) -> Result<(), EndpointBindingDriverError> {
    let decision = row.source();
    let right = row.attachment().requested_rights();
    if !decision.admitted_rights().contains(&right) {
        return Err(
            EndpointBindingDriverError::new(EndpointBindingDriverErrorKind::RightsNotAdmitted, op)
                .with_detail(FailureDetail::at("spec/source-decision").comparison(
                    FailureComparison::new(
                        "source.admittedRights",
                        right_name(right),
                        "another right set",
                    ),
                )),
        );
    }
    if let Some(facet) = row
        .attachment()
        .required_facets()
        .iter()
        .find(|facet| !decision.realized_facets().contains(facet))
    {
        return Err(
            EndpointBindingDriverError::new(EndpointBindingDriverErrorKind::FacetUnsupported, op)
                .with_detail(FailureDetail::at("spec/source-decision").comparison(
                    FailureComparison::new(
                        "source.realizedFacets",
                        "the delivery's required facet",
                        format!("{facet:?}"),
                    ),
                )),
        );
    }
    Ok(())
}

/// The stable name one requested right renders as in a failure comparison.
const fn right_name(right: RequestedRights) -> &'static str {
    match right {
        RequestedRights::Observe => "observe",
        RequestedRights::Consume => "consume",
        RequestedRights::Mutate => "mutate",
        RequestedRights::Share => "share",
        RequestedRights::Exclusive => "exclusive",
    }
}

// ---------------------------------------------------------------------------
// The delivery this row owes its consumer
// ---------------------------------------------------------------------------

/// How one committed row's delivery reaches its consumer.
///
/// The form is derived from the row's own attachment kind, never from a
/// caller: a `connect` or `listen` is delivered as a verified descriptor, and
/// an `attach` - which addresses a display or a stream by name - is delivered
/// as a private exact-socket presentation at the one destination component
/// this relationship owns inside the consumer's own launch tree.
///
/// The descriptor slot a delivery lands in belongs to the consumer's launch,
/// not to this row: a binding row has no descriptor field and never invents
/// one. The effect adapter the daemon supplies owns that slot and builds the
/// concrete delivery, so the two forms carry exactly the authority each of
/// them needs and neither can be widened into the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointBindingDelivery {
    form: DeliveryForm,
    destination: Option<BoundedToken>,
}

impl EndpointBindingDelivery {
    /// Derive the delivery one committed row declares.
    ///
    /// The destination component is the row's own stable consumer slot, which
    /// the bounded-token grammar confines to `[a-z][a-z0-9-]{0,62}`: a slot
    /// can therefore neither be an absolute path nor carry a relative
    /// component, so the destination this row owns is a slot and never a
    /// path. A descriptor delivery owns no destination at all, which is why
    /// it can never be widened into one.
    pub fn for_row(row: &EndpointBindingSpec) -> Self {
        let form = declared_delivery_form(*row.attachment());
        Self {
            form,
            destination: match form {
                DeliveryForm::Descriptor => None,
                DeliveryForm::PrivateSocketPresentation => Some(row.slot().clone()),
            },
        }
    }

    /// The closed delivery form this row's attachment kind declares.
    pub const fn form(&self) -> DeliveryForm {
        self.form
    }

    /// The single destination component this relationship owns, when it
    /// owns one.
    pub fn destination(&self) -> Option<&BoundedToken> {
        self.destination.as_ref()
    }

    /// Whether this delivery is the form `attachment` declares.
    pub const fn satisfies(&self, attachment: EndpointAttachmentKind) -> bool {
        self.form.satisfies(attachment)
    }

    /// The realization facets this delivery form depends on.
    ///
    /// A presentation delivers the same inode as a descriptor does, so it
    /// needs the descriptor facet too; only the name-carrying form adds the
    /// private-pathname facet.
    pub const fn required_facets(&self) -> &'static [BindingRealizationFacet] {
        match self.form {
            DeliveryForm::Descriptor => &[BindingRealizationFacet::EndpointDescriptor],
            DeliveryForm::PrivateSocketPresentation => &[
                BindingRealizationFacet::EndpointDescriptor,
                BindingRealizationFacet::EndpointPathname,
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// The delivery target the daemon realizes
// ---------------------------------------------------------------------------

/// Everything the effect adapter needs to reach ONE exact endpoint for ONE
/// admitted consumer, and nothing more.
///
/// Every field is a committed-row fact: the Zone the row lives in, the exact
/// `Endpoint` reference and the store-assigned identity and generation the
/// owner published, the admitted consumer, the stable consumer slot, the
/// attachment kind, and the bounded purpose. There is no host path, no
/// socket name, no numeric host principal, and no locator of any kind here:
/// the endpoint's locator stays with the effect adapter, which resolves it
/// privately from the committed `Endpoint` row.
#[derive(Clone, PartialEq, Eq)]
pub struct EndpointDeliveryTarget {
    zone: ZoneId,
    endpoint_ref: ResourceRef,
    endpoint_uid: ResourceUid,
    endpoint_generation: ResourceGeneration,
    consumer_ref: ResourceRef,
    slot: BoundedToken,
    attachment: EndpointAttachmentKind,
    purpose: BoundedToken,
}

impl EndpointDeliveryTarget {
    /// Assemble the target from the row's own committed facts.
    pub(crate) fn derive(
        zone: &ZoneId,
        row: &EndpointBindingSpec,
        endpoint_uid: ResourceUid,
        endpoint_generation: ResourceGeneration,
        purpose: &BoundedToken,
    ) -> Self {
        Self {
            zone: zone.clone(),
            endpoint_ref: row.endpoint_ref().clone(),
            endpoint_uid,
            endpoint_generation,
            consumer_ref: row.execution_ref().clone(),
            slot: row.slot().clone(),
            attachment: *row.attachment(),
            purpose: purpose.clone(),
        }
    }

    /// Borrow the Zone this relationship lives in.  A binding is a same-Zone
    /// relationship; nothing here reaches another Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the exact `Endpoint` reference this row delivers.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the store-assigned identity of the exact endpoint.
    pub const fn endpoint_uid(&self) -> &ResourceUid {
        &self.endpoint_uid
    }

    /// Return the endpoint generation this relationship is bound to.
    pub const fn endpoint_generation(&self) -> ResourceGeneration {
        self.endpoint_generation
    }

    /// Borrow the admitted consumer.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the stable consumer slot the delivery occupies.
    pub const fn slot(&self) -> &BoundedToken {
        &self.slot
    }

    /// Return the attachment kind the consumer requested.
    pub const fn attachment(&self) -> EndpointAttachmentKind {
        self.attachment
    }

    /// Borrow the bounded usage purpose.
    pub const fn purpose(&self) -> &BoundedToken {
        &self.purpose
    }
}

impl core::fmt::Debug for EndpointDeliveryTarget {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EndpointDeliveryTarget")
            .field("zone", &self.zone)
            .field("endpoint_ref", &self.endpoint_ref)
            .field("endpoint_uid", &self.endpoint_uid)
            .field("endpoint_generation", &self.endpoint_generation)
            .field("consumer_ref", &self.consumer_ref)
            .field("slot", &self.slot)
            .field("attachment", &self.attachment)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointBindingDriverErrorKind {
    /// The durable spec did not decode as the strict neutral binding
    /// contract, or its source selector does not name one exact endpoint.
    SpecInvalid,
    /// The spec selects a Provider this driver does not own.
    ProviderUnsupported,
    /// The named `Endpoint` row is present but its owner uid differs from
    /// this row's owner: the manager would silently re-parent. Terminal -
    /// the committed rows cannot converge by retrying.
    OwnerMismatch,
    /// The named `Endpoint` row is not observable yet: the manager answered
    /// `Absent` (the row may simply not be committed yet) or could not answer
    /// at all. Retryable by contract (issue #511).
    EndpointUnavailable,
    /// The named `Endpoint` row is present but is not a usable endpoint (its
    /// uid or stored spec does not decode). Terminal: the committed row
    /// cannot converge by retrying.
    EndpointSpecInvalid,
    /// The endpoint's own declaration refuses this consumer, this operation,
    /// or one more attachment.
    EndpointRefused,
    /// The row's committed source decision does not admit the right this
    /// row's attachment kind claims.
    RightsNotAdmitted,
    /// The row's committed source decision does not declare a realization
    /// facet this row's delivery depends on.
    FacetUnsupported,
    /// A provider serving effect failed transiently.
    ServingEffect,
    /// The consumer is still attached to the delivered endpoint, so the
    /// teardown may not cut it loose.
    DrainPending,
}

impl EndpointBindingDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::ServingEffect | Self::EndpointUnavailable | Self::DrainPending => {
                FailureClass::Retryable
            }
            Self::SpecInvalid
            | Self::ProviderUnsupported
            | Self::OwnerMismatch
            | Self::EndpointSpecInvalid
            | Self::EndpointRefused
            | Self::RightsNotAdmitted
            | Self::FacetUnsupported => FailureClass::Terminal,
        }
    }

    /// The registered failure kind this classification reports (issue #508).
    const fn failure_kind(self) -> FailureKind {
        match self {
            // A source decision that withholds the right or the facet is a
            // committed-row refusal, not an operational failure.
            Self::SpecInvalid | Self::RightsNotAdmitted | Self::FacetUnsupported => {
                FailureKinds::BINDING_SPEC_INVALID
            }
            Self::ProviderUnsupported => FailureKinds::BINDING_PROVIDER_UNSUPPORTED,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::EndpointUnavailable => FailureKinds::BINDING_PARENT_UNAVAILABLE,
            Self::EndpointSpecInvalid => FailureKinds::BINDING_PARENT_SPEC_INVALID,
            Self::EndpointRefused => FailureKinds::BINDING_PLAN_DERIVATION_INVALID,
            Self::ServingEffect => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
            // The shared drain kind: an outstanding consumer is not a
            // serving-effect failure.
            Self::DrainPending => FailureKinds::CHILDREN_DRAINING,
        }
    }
}

/// Typed driver failure; mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`] (R13, issue
/// #508).
#[derive(Debug, Clone)]
pub(crate) struct EndpointBindingDriverError {
    kind: EndpointBindingDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl EndpointBindingDriverError {
    fn new(kind: EndpointBindingDriverErrorKind, op: DriverOp) -> Self {
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

impl core::fmt::Display for EndpointBindingDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            EndpointBindingDriverErrorKind::SpecInvalid => "binding-spec-invalid",
            EndpointBindingDriverErrorKind::ProviderUnsupported => "binding-provider-unsupported",
            EndpointBindingDriverErrorKind::OwnerMismatch => "binding-owner-mismatch",
            EndpointBindingDriverErrorKind::EndpointUnavailable => "binding-parent-unavailable",
            EndpointBindingDriverErrorKind::EndpointSpecInvalid => "binding-parent-spec-invalid",
            EndpointBindingDriverErrorKind::EndpointRefused => "endpoint-policy-refused",
            EndpointBindingDriverErrorKind::RightsNotAdmitted => "endpoint-rights-not-admitted",
            EndpointBindingDriverErrorKind::FacetUnsupported => "endpoint-facet-not-admitted",
            EndpointBindingDriverErrorKind::ServingEffect => "binding-serving-effect-failed",
            EndpointBindingDriverErrorKind::DrainPending => "children-draining",
        })
    }
}

impl std::error::Error for EndpointBindingDriverError {}

/// Typed in-memory status projection (R11: never persisted).
///
/// The status carries the pinned identity of the exact endpoint the row's
/// delivery names, never a path: the `Debug` rendering of
/// [`EndpointSocketIdentity`] carries the device and inode only, so a log
/// line built from this value cannot leak the endpoint's host location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndpointBindingDriverStatus {
    /// The exact endpoint was verified this pass and the delivery names it.
    Delivering {
        form: DeliveryForm,
        socket: EndpointSocketIdentity,
        /// The delivery was (re)issued this pass rather than already current.
        issued: bool,
    },
    /// The exact endpoint could not be proven reachable for this consumer
    /// this pass, so nothing was delivered. The stable reason stays visible
    /// instead of collapsing into a generic error (KTD5).
    Unproven {
        form: DeliveryForm,
        reason: &'static str,
    },
    /// The exact pre-restart endpoint was re-verified and re-adopted without
    /// a fresh delivery.
    Recovered {
        form: DeliveryForm,
        socket: EndpointSocketIdentity,
    },
    /// A terminal admission rejection, kept in memory while the actor
    /// publishes the Failed phase.
    Rejected { reason: &'static str },
}

/// The fence one observation of this row was published under.
///
/// The UID pins reassignment, the generation pins spec changes, and the
/// revision names the row revision the evidence was observed at. The manager
/// plane carries no Zone revision and maps the row generation onto the wire
/// revision (KTD8), so a projection pinned to any other ordinal would either
/// claim an observation the row never held or fall behind the row it
/// describes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndpointBindingReadinessFence {
    /// The binding UID the evidence was observed under.
    pub uid: ResourceUid,
    /// The binding spec generation the evidence was observed under.
    pub generation: ResourceGeneration,
    /// The row revision the evidence was observed at.
    pub revision: ZoneRevision,
}

impl EndpointBindingReadinessFence {
    /// Whether this fence still matches the row's current identity.
    pub fn matches(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.uid == *uid && self.generation == generation && self.revision <= revision
    }
}

/// The public `EndpointBinding` status projection this driver publishes.
///
/// It exposes fenced readiness and a stable safe failure code, and nothing
/// else: the endpoint's locator, the pinned `(dev, ino)`, the descriptor
/// slot, and the consumer's launch tree stay provider-private and never
/// appear here. A consumer learns only whether the exact endpoint it named
/// is being delivered to it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndpointBindingStatusResource {
    /// Whether the serving side reports the exact endpoint delivered.
    pub ready: bool,
    /// The fence the readiness evidence was observed under.
    pub fence: EndpointBindingReadinessFence,
    /// The stable safe failure code, when not ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<StatusCode>,
}

impl EndpointBindingStatusResource {
    /// Whether this projection reports ready under the current fence.
    ///
    /// Readiness without a matching UID and generation is never current
    /// (fail-closed). The fence revision only needs to precede the stored
    /// revision: the status write carrying the report advances the row past
    /// the observed commit.
    pub fn readiness_is_current(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.ready && self.fence.matches(uid, generation, revision)
    }
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one `EndpointBinding` row, exactly as
/// persisted: the selected Provider and the type-specific base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingSpecEnvelope {
    provider_ref: Option<ResourceRef>,
    base: CanonicalJsonObject,
}

/// The manager-wired decode hook for `EndpointBinding` rows.
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

/// The provider-facing serving effect surface the binding driver needs.
///
/// Every method answers for ONE exact endpoint and ONE admitted consumer, and
/// every input is a committed-row fact ([`EndpointDeliveryTarget`]); none of
/// them accepts a host path, a socket name, or a locator, because the
/// endpoint's locator stays private to the effect adapter that resolves it
/// from the committed `Endpoint` row. The driver therefore never mutates host
/// state itself: verification, delivery, fencing, and release all cross the
/// provider boundary through this port, and the production implementation
/// lives in the daemon behind it (R4).
#[async_trait::async_trait]
pub trait EndpointBindingDriverEffects: Send + Sync + 'static {
    /// What the kernel actually applies for this consumer on this exact
    /// endpoint: the pinned `(dev, ino)` the owner resolved, the effective
    /// rights (the named ACL entry already ANDed with the mask, or the inode's
    /// mode class), the effective traverse bit every ancestor applies, whether
    /// the containing directory is listable, and whether the endpoint is
    /// accepting.
    ///
    /// This is a read: the driver calls it on every pass and on recover, and
    /// a pass that cannot obtain an observation delivers nothing.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot complete the
    /// observation. The observation itself never fails open: an endpoint the
    /// adapter cannot resolve answers with the shortest effective access the
    /// [`EndpointAccessObservation`] contract allows, so the driver's checks
    /// refuse it.
    async fn verify(&self, target: &EndpointDeliveryTarget)
        -> Result<EndpointAccessObservation, String>;

    /// Deliver the exact endpoint to the consumer in the declared form and
    /// answer the identity of the endpoint the delivery actually names.
    ///
    /// The adapter owns the launch's descriptor slot and builds the concrete
    /// delivery; the row decides only WHICH form and WHICH destination
    /// component the relationship owns.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the delivery could not be established. The call is
    /// idempotent under retry: a delivery that is already in place for this
    /// target answers with the same pinned identity and re-issues nothing.
    async fn deliver(
        &self,
        target: &EndpointDeliveryTarget,
        delivery: &EndpointBindingDelivery,
    ) -> Result<EndpointSocketIdentity, String>;

    /// Block new use of this relationship ahead of its typed release.
    ///
    /// The pre-drain step: no further delivery is established through this
    /// row while the consumer finishes what it already holds. Idempotent
    /// under retry.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot fence the
    /// relationship; the teardown does not proceed on an unfenced row.
    async fn fence(&self, target: &EndpointDeliveryTarget) -> Result<(), String>;

    /// Whether the consumer is still attached to the delivered endpoint.
    ///
    /// This is the teardown gate (U13's rule in the endpoint family's own
    /// terms): a relationship whose consumer still holds the exact endpoint
    /// keeps the durable deleting mark and its delivery, and the pass
    /// retries rather than pulling the endpoint out from under live use.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot complete the
    /// observation. The observation fails closed: an answer the adapter
    /// cannot establish is `true` (still attached), never `false`.
    async fn consumer_attached(&self, target: &EndpointDeliveryTarget) -> Result<bool, String>;

    /// Release this relationship's delivery.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter fails to release the
    /// delivery. The release is idempotent under retry: a delivery that was
    /// never established, or whose endpoint is already gone, answers
    /// `Ok(())`.
    async fn release(&self, target: &EndpointDeliveryTarget) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate the binding driver
/// factory for one zone: the serving effects plus nothing else. The zone
/// authority the endpoint family owns (its locator, its descriptor slots)
/// stays inside the effect adapter those effects are built from, so no
/// daemon-built port appears at any construction site (R2).
pub struct EndpointBindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: ZoneId,
    /// The daemon-supplied facet set the family's effects are built from
    /// (R2): the exact endpoint verification, the delivery, the fence, the
    /// attachment observation, and the release.
    pub facets: crate::facets::EndpointBindingEffectFacets,
}

/// [`ResourceDriverFactory`] for the `EndpointBinding` resource type.
/// Construction is infallible by contract (R3).
pub(crate) struct EndpointBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: EndpointBindingDriverArgs,
}

impl EndpointBindingDriverFactory {
    pub(crate) fn new(args: EndpointBindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(ENDPOINT_BINDING_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for EndpointBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(EndpointBindingDriver::new(
            self.args.zone.clone(),
            // The driver builds its effects from the declared facets; no
            // externally built port appears at this construction site (R2).
            Arc::new(crate::effects_service::EndpointBindingEffectsService::new(
                self.args.facets.clone(),
            )),
        ))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// The exact endpoint row one committed relationship is bound to.
#[derive(Debug, Clone)]
struct ResolvedEndpoint {
    uid: ResourceUid,
    generation: ResourceGeneration,
    spec: EndpointSpec,
}

/// One `EndpointBinding` resource's driver.
pub(crate) struct EndpointBindingDriver {
    zone: ZoneId,
    effects: Arc<dyn EndpointBindingDriverEffects>,
    /// The pinned identity of the exact endpoint this row's live delivery
    /// names, once one has been established (R6/R11): runtime-only, so a
    /// restart re-verifies rather than trusting a cached identity.
    delivered: Option<EndpointSocketIdentity>,
    /// Targets this driver already registered a dependency watch on
    /// (R12/R17). Runtime-only (R6/R11): one registration per target keeps
    /// the dependency edge that wakes the actor on dependency death or
    /// readiness without accumulating manager watch entries.
    watched: Vec<ResourceKey>,
}

impl EndpointBindingDriver {
    pub(crate) fn new(zone: ZoneId, effects: Arc<dyn EndpointBindingDriverEffects>) -> Self {
        Self {
            zone,
            effects,
            delivered: None,
            watched: Vec::new(),
        }
    }

    fn error(
        &self,
        kind: EndpointBindingDriverErrorKind,
        op: DriverOp,
    ) -> EndpointBindingDriverError {
        EndpointBindingDriverError::new(kind, op)
    }

    /// Decode the stored envelope and hand the committed row over.
    ///
    /// The row itself is read by [`committed_row`], the one seam that names
    /// a committed row's field set. The serving Provider is checked here: a
    /// row that names a Provider outside this family is refused, and only an
    /// absent `providerRef` is served by the declaring Provider.
    fn decoded_binding(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<EndpointBindingSpec, EndpointBindingDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(EndpointBindingDriverErrorKind::SpecInvalid, op))?;
        if let Some(provider_ref) = envelope.provider_ref.as_ref()
            && provider_ref.to_canonical_string() != BINDING_PROVIDER_REF
        {
            return Err(self
                .error(EndpointBindingDriverErrorKind::ProviderUnsupported, op)
                .with_detail(FailureDetail::at("spec/provider").comparison(
                    FailureComparison::new(
                        "spec.providerRef",
                        BINDING_PROVIDER_REF,
                        provider_ref.to_canonical_string(),
                    ),
                )));
        }
        let base = serde_json::from_slice::<serde_json::Value>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(EndpointBindingDriverErrorKind::SpecInvalid, op))?;
        committed_row(&base, op)
    }

    /// The key of the exact `Endpoint` this binding names.
    fn endpoint_key(&self, row: &EndpointBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            ENDPOINT_TYPE_NAME,
            row.endpoint_ref().name().as_str(),
        )
    }

    /// The key of the admitted consumer this binding names.
    fn consumer_key(&self, row: &EndpointBindingSpec) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            row.execution_ref().resource_type().as_str(),
            row.execution_ref().name().as_str(),
        )
    }

    /// The exact `Endpoint` row through the manager (R2: the driver never
    /// touches the spec store).
    ///
    /// The named endpoint must be the row the manager reports as this
    /// resource's owner - a child cannot silently change owner. Classified
    /// per issue #511: a row that is not observable yet (`Absent`,
    /// `Unavailable`, or an `Error` the manager could not answer with a
    /// usable row) defers retryably, so an endpoint that simply has not been
    /// committed yet never fails the binding terminal; `OwnerMismatch`
    /// applies only to a present row whose owner uid actually differs.
    async fn endpoint_row(
        &self,
        ctx: &mut ResourceContext,
        row: &EndpointBindingSpec,
        op: DriverOp,
    ) -> Result<ResolvedEndpoint, EndpointBindingDriverError> {
        let key = self.endpoint_key(row);
        let lookup = ctx.lookup(&key).await;
        let row = match lookup {
            RowLookup::Present { row, .. } => row,
            _ => {
                // A non-present read defers: the row may not be committed yet
                // and an unusable payload is not terminal by itself (#511).
                let mut detail = FailureDetail::at("endpoint/lookup");
                if let Some(comparison) =
                    lookup.failure_comparison("endpoint.row", "present")
                {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                if let RowLookup::Error { plane, detail: error_detail } = &lookup {
                    tracing::warn!(
                        plane = ?plane,
                        key = %key,
                        detail = %error_detail,
                        "endpoint binding endpoint row read answered with an unreadable row",
                    );
                }
                return Err(self
                    .error(EndpointBindingDriverErrorKind::EndpointUnavailable, op)
                    .with_detail(detail));
            }
        };
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            // The row's declared owner uid does not match the endpoint the
            // spec names: refuse rather than silently re-parent (R8).
            return Err(self
                .error(EndpointBindingDriverErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("endpoint/owner").comparison(
                    FailureComparison::new(
                        "endpoint.ownerUid",
                        uid_hex(owner),
                        uid_hex(&row.uid),
                    ),
                )));
        }
        let uid = resource_uid(&row.uid).map_err(|_| self.endpoint_spec_invalid(op, "endpoint.uid"))?;
        let generation =
            ResourceGeneration::new(row.generation).map_err(|_| {
                self.endpoint_spec_invalid(op, "endpoint.generation")
            })?;
        let envelope = serde_json::from_slice::<ResourceSpec>(&row.spec)
            .map_err(|_| self.endpoint_spec_invalid(op, "endpoint.spec"))?;
        // `EndpointSpec` validates `providerRef` as part of its typed
        // contract, so the envelope layer is folded back onto the base
        // before the parse.
        let spec = serde_json::from_slice::<EndpointSpec>(
            &envelope.base_with_provider_ref().to_canonical_bytes(),
        )
        .map_err(|_| self.endpoint_spec_invalid(op, "endpoint.spec"))?;
        Ok(ResolvedEndpoint {
            uid,
            generation,
            spec,
        })
    }

    /// The terminal classification for a present endpoint row whose stored
    /// identity or spec does not decode (issue #508: this is not an
    /// ownership mismatch).
    fn endpoint_spec_invalid(
        &self,
        op: DriverOp,
        field: &'static str,
    ) -> EndpointBindingDriverError {
        self.error(EndpointBindingDriverErrorKind::EndpointSpecInvalid, op)
            .with_detail(FailureDetail::at("endpoint/decode").comparison(
                FailureComparison::new(field, "a canonical Endpoint row", "decode failed"),
            ))
    }

    /// Apply the endpoint's OWN declaration to this consumer's request.
    ///
    /// The decision is the Endpoint provider's, not this driver's: the
    /// consumer must be on the endpoint's subject allowlist, the operation
    /// the attachment kind performs on the endpoint's operation allowlist, an
    /// `attach` needs the endpoint's own attachment ceiling, and every facet
    /// the request depends on must be one the endpoint family realizes. An
    /// empty allowlist is the deliberately unconstrained policy, so it admits
    /// rather than denies. A refusal here is terminal: the endpoint's own
    /// rules are the endpoint's to change, and retrying cannot make this
    /// consumer an admitted one.
    fn endpoint_admits(
        &self,
        ctx: &mut ResourceContext,
        spec: &EndpointSpec,
        row: &EndpointBindingSpec,
        op: DriverOp,
    ) -> Result<(), EndpointBindingDriverError> {
        match endpoint_policy_refusal(spec, row) {
            Some(reason) => Err(self.rejected(ctx, reason, op)),
            None => Ok(()),
        }
    }

    /// Record one terminal admission rejection in this pass's in-memory
    /// status (old `failed_binding_result`) and return the typed failure: the
    /// actor publishes the Failed phase, and the stable provider reason stays
    /// visible instead of collapsing into a generic error (KTD5).
    fn rejected(
        &self,
        ctx: &mut ResourceContext,
        reason: EndpointBindingError,
        op: DriverOp,
    ) -> EndpointBindingDriverError {
        ctx.set_status(EndpointBindingDriverStatus::Rejected {
            reason: endpoint_error_code(reason),
        });
        self.error(EndpointBindingDriverErrorKind::EndpointRefused, op)
            .with_detail(FailureDetail::at("endpoint/admit").with_note(reason.to_string()))
    }

    /// The delivery target for one committed request and the exact endpoint
    /// row it was bound to.
    fn delivery_target(
        &self,
        row: &EndpointBindingSpec,
        endpoint: &ResolvedEndpoint,
    ) -> EndpointDeliveryTarget {
        EndpointDeliveryTarget::derive(
            &self.zone,
            row,
            endpoint.uid.clone(),
            endpoint.generation,
            endpoint.spec.purpose(),
        )
    }

    /// What the freshly observed endpoint prevents this pass from
    /// delivering, if anything.
    ///
    /// The check is the source family's rule: the effective access must cover
    /// the bits the admitted right needs, every ancestor must still apply a
    /// traverse bit, the containing directory must NOT be listable (listing
    /// it is the authority the family exists to remove, and a directory that
    /// happens to be traversable is not a grant), and an `attach` needs an
    /// endpoint that is accepting. A replaced inode is handled by the caller,
    /// which compares the observed identity against the pinned one.
    fn unproven(
        observation: &EndpointAccessObservation,
        row: &EndpointBindingSpec,
    ) -> Option<&'static str> {
        if !observation.grants(required_right_bits(row.attachment().requested_rights()))
            || !observation.traversable()
        {
            return Some(ENDPOINT_NOT_READY);
        }
        if observation.parent_listable() {
            return Some(ENDPOINT_DIRECTORY_LISTABLE);
        }
        if *row.attachment() == EndpointAttachmentKind::Attach && !observation.accepting() {
            return Some(ENDPOINT_NOT_READY);
        }
        None
    }

    /// The fence this row's projection is pinned to: the row's own identity
    /// at the row revision the manager publishes (the row generation, which
    /// the manager maps onto the wire revision, KTD8).
    fn fence(&self, ctx: &ResourceContext) -> Result<EndpointBindingReadinessFence, EndpointBindingDriverError> {
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(EndpointBindingDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(EndpointBindingDriverErrorKind::SpecInvalid, DriverOp::Reconcile))?;
        Ok(EndpointBindingReadinessFence {
            uid,
            generation,
            revision: ZoneRevision::new(generation.get()),
        })
    }

    /// Publish the fenced readiness projection for this pass.
    ///
    /// The fence names the row's own identity, so the read side can tell a
    /// current report from one authored under a different identity or ahead
    /// of the stored revision. A pass that proves nothing publishes
    /// `ready: false` under the stable code rather than no projection at
    /// all, so a cleared projection layer can never be read as ready.
    fn publish_projection(
        &self,
        ctx: &mut ResourceContext,
        ready: bool,
        reason: Option<&'static str>,
    ) {
        let Ok(fence) = self.fence(ctx) else {
            return;
        };
        let projection = EndpointBindingStatusResource {
            ready,
            fence,
            reason: reason.map(|code| {
                StatusCode::parse(code).expect("the driver's stable codes are valid status codes")
            }),
        };
        ctx.set_status_projection(
            serde_json::to_value(projection)
                .expect("the fenced binding projection is always serializable"),
        );
    }

    /// Register one dependency watch (R12/R17) exactly once per target.
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet (`register_watch` refuses it), and the
    /// resync requeue re-evaluates those rows until they are served.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// Observe the exact endpoint through the effect port and report the
    /// stable code for whatever the observation does not yet prove.
    async fn verified_observation(
        &self,
        target: &EndpointDeliveryTarget,
        op: DriverOp,
    ) -> Result<EndpointAccessObservation, EndpointBindingDriverError> {
        self.effects.verify(target).await.map_err(|error| {
            self.error(EndpointBindingDriverErrorKind::ServingEffect, op).with_detail(
                FailureDetail::at("endpoint/verify")
                    .comparison(FailureComparison::new(
                        "endpoint.observation",
                        "an observation of the exact endpoint",
                        "the adapter failed",
                    ))
                    .with_note(error),
            )
        })
    }
}

fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    ResourceUid::from_bytes(bytes).map_err(|_| ())
}

/// The hex spelling one compared uid renders as (issue #508).
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The endpoint's own refusal of one consumer's request, in the Endpoint
/// provider's own vocabulary.
///
/// Every branch is a refusal: there is no branch that admits a consumer the
/// endpoint's declaration does not name, performs an operation it never
/// declared, or takes one attachment more than the endpoint's own ceiling
/// allows.
fn endpoint_policy_refusal(
    spec: &EndpointSpec,
    row: &EndpointBindingSpec,
) -> Option<EndpointBindingError> {
    let policy = spec.consumer_policy();
    if !policy.admits_subject(row.execution_ref()) {
        return Some(EndpointBindingError::ConsumerNotAllowed);
    }
    if !policy.admits_operation(EndpointConsumerPolicy::operation_for(*row.attachment())) {
        return Some(EndpointBindingError::OperationNotAllowed);
    }
    if *row.attachment() == EndpointAttachmentKind::Attach
        && !spec.attachment_policy().admits_attachment(0)
    {
        return Some(EndpointBindingError::AttachmentRefused);
    }
    // The IMPLEMENTATION's own capability, which is a different question
    // from the row's committed source decision (`admitted_source_decision`):
    // this asks whether the endpoint family can realize the facet at all,
    // where the decision asks whether the SOURCE admitted this relationship
    // through it. Both are required.
    let support = endpoint_binding_support();
    if !row
        .attachment()
        .required_facets()
        .iter()
        .all(|facet| support.realizes(*facet))
    {
        return Some(EndpointBindingError::UnsupportedFacet);
    }
    // The delivery this row would construct is exactly the form its
    // attachment kind declares; a mismatch is refused rather than
    // approximated with the other form.
    if !EndpointBindingDelivery::for_row(row).satisfies(*row.attachment()) {
        return Some(EndpointBindingError::UnsupportedFacet);
    }
    None
}

/// The stable code one endpoint-policy refusal reports.
///
/// The codes are field-free: they name the class of refusal, never the
/// socket, the consumer, or the host bytes it was protecting.
const fn endpoint_error_code(reason: EndpointBindingError) -> &'static str {
    match reason {
        EndpointBindingError::ConsumerNotAllowed => "endpoint-consumer-not-allowed",
        EndpointBindingError::ComponentNotAllowed => "endpoint-component-not-allowed",
        EndpointBindingError::OperationNotAllowed => "endpoint-operation-not-allowed",
        EndpointBindingError::AttachmentRefused => "endpoint-attachment-refused",
        EndpointBindingError::TargetSupportMissing => "endpoint-target-support-missing",
        EndpointBindingError::UnsupportedFacet => "endpoint-facet-unsupported",
        _ => "endpoint-policy-refused",
    }
}

/// The stable name one delivery form renders as in a failure comparison.
const fn delivery_form_name(form: DeliveryForm) -> &'static str {
    match form {
        DeliveryForm::Descriptor => "descriptor",
        DeliveryForm::PrivateSocketPresentation => "private-socket-presentation",
    }
}

#[async_trait::async_trait]
impl ResourceDriver for EndpointBindingDriver {
    type Error = EndpointBindingDriverError;

    fn classify_error(&self, error: &EndpointBindingDriverError) -> DriverFailure {
        let failure = match error.kind {
            EndpointBindingDriverErrorKind::SpecInvalid
            | EndpointBindingDriverErrorKind::ProviderUnsupported
            | EndpointBindingDriverErrorKind::OwnerMismatch
            | EndpointBindingDriverErrorKind::EndpointSpecInvalid
            | EndpointBindingDriverErrorKind::EndpointRefused
            | EndpointBindingDriverErrorKind::RightsNotAdmitted
            | EndpointBindingDriverErrorKind::FacetUnsupported => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            EndpointBindingDriverErrorKind::EndpointUnavailable
            | EndpointBindingDriverErrorKind::DrainPending => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            EndpointBindingDriverErrorKind::ServingEffect => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Structural validation (`validate_spec`): the exact-endpoint selector
    /// rule, the named `Endpoint` row, and the owner fence.
    ///
    /// A row that names an endpoint this Zone does not hold, or one the
    /// manager cannot answer for, defers retryably - the endpoint may simply
    /// not be committed yet (issue #511). A row that names an endpoint the
    /// manager reports with a different owner uid, or one whose stored spec
    /// does not decode, is terminal: the committed rows cannot converge by
    /// retrying.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Validate;
        let row = self.decoded_binding(ctx, op)?;
        let endpoint = self.endpoint_row(ctx, &row, op).await?;
        self.endpoint_admits(ctx, &endpoint.spec, &row, op)?;
        Ok(())
    }

    /// Adoption after a restart (`observe`): the exact endpoint is VERIFIED
    /// and re-adopted, and never re-delivered.
    ///
    /// The pinned identity is runtime-only, so a restart has nothing cached
    /// to compare: adoption is therefore a fresh observation of the named
    /// endpoint, and a row whose endpoint is absent, undecodable, or no
    /// longer provable for this consumer adopts nothing and is re-delivered
    /// by the next reconcile pass.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let row = self.decoded_binding(ctx, op)?;
        let Ok(endpoint) = self.endpoint_row(ctx, &row, op).await else {
            return Ok(RecoveryOutcome::Missing);
        };
        if self.endpoint_admits(ctx, &endpoint.spec, &row, op).is_err() {
            return Ok(RecoveryOutcome::Missing);
        }
        let target = self.delivery_target(&row, &endpoint);
        let observation = self.verified_observation(&target, op).await?;
        if Self::unproven(&observation, &row).is_some() {
            return Ok(RecoveryOutcome::Missing);
        }
        let delivery = EndpointBindingDelivery::for_row(&row);
        ctx.set_status(EndpointBindingDriverStatus::Recovered {
            form: delivery.form(),
            socket: observation.socket(),
        });
        self.delivered = None;
        Ok(RecoveryOutcome::Adopted)
    }

    /// One reconcile pass: bind the exact endpoint, apply its own policy,
    /// verify what the kernel actually applies, and deliver only what the
    /// verification proves.
    ///
    /// The pass never assumes the endpoint is reachable. An observation that
    /// does not prove the effective access, the ancestor traverse bits, the
    /// non-listable containing directory, or - for an `attach` - an accepting
    /// endpoint delivers nothing, publishes the stable not-ready code, and
    /// re-checks on the preserved cadence. An observation whose inode differs
    /// from the pinned one means the endpoint was replaced underneath this
    /// row: the stale delivery is dropped and the NEW exact endpoint is
    /// delivered in its place.
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let row = self.decoded_binding(ctx, op)?;
        let endpoint = self.endpoint_row(ctx, &row, op).await?;
        // Dependency edges (R12/R17): the exact endpoint and the admitted
        // consumer wake this actor when either changes or dies.
        self.watch_once(ctx, self.endpoint_key(&row)).await;
        self.watch_once(ctx, self.consumer_key(&row)).await;
        self.endpoint_admits(ctx, &endpoint.spec, &row, op)?;
        let delivery = EndpointBindingDelivery::for_row(&row);
        let target = self.delivery_target(&row, &endpoint);
        let observation = self.verified_observation(&target, op).await?;

        if let Some(reason) = Self::unproven(&observation, &row) {
            self.delivered = None;
            ctx.set_status(EndpointBindingDriverStatus::Unproven {
                form: delivery.form(),
                reason,
            });
            self.publish_projection(ctx, false, Some(reason));
            ctx.requeue_after(ENDPOINT_BINDING_RESYNC);
            // The pass converged its own work; the serving half rides the
            // fenced projection, which reports `ready: false` under the
            // stable code until the exact endpoint is proven.
            return Ok(ReconcileOutcome::Satisfied);
        }

        // A replaced inode invalidates the cached delivery: the relationship
        // stops reporting the endpoint that was there before and re-delivers
        // against the NEW exact endpoint, or reports degraded.
        let replaced = self
            .delivered
            .is_some_and(|pinned| pinned != observation.socket());
        if replaced {
            tracing::info!(
                endpoint = %target.endpoint_ref(),
                consumer = %target.consumer_ref(),
                reason = ENDPOINT_IDENTITY_REPLACED,
                "the exact endpoint this row delivered was replaced; re-delivering"
            );
            self.delivered = None;
        }
        if self.delivered.is_none() {
            let socket = self
                .effects
                .deliver(&target, &delivery)
                .await
                .map_err(|error| {
                    self.error(EndpointBindingDriverErrorKind::ServingEffect, op).with_detail(
                        FailureDetail::at("endpoint/deliver")
                            .comparison(FailureComparison::new(
                                "endpoint.delivery",
                                delivery_form_name(delivery.form()),
                                "the adapter failed",
                            ))
                            .with_note(error),
                    )
                })?;
            self.delivered = Some(socket);
            ctx.set_status(EndpointBindingDriverStatus::Delivering {
                form: delivery.form(),
                socket,
                issued: true,
            });
        } else {
            ctx.set_status(EndpointBindingDriverStatus::Delivering {
                form: delivery.form(),
                socket: observation.socket(),
                issued: false,
            });
        }
        self.publish_projection(ctx, true, None);
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Pre-drain: block new use for this relationship before anything is
    /// torn down.
    ///
    /// The step fences the row's own use - no further delivery is established
    /// through it - and is idempotent under retry, so a requeued pass
    /// converges. It never waits on a descendant.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        // A row whose spec no longer decodes delivered nothing and has
        // nothing to fence.
        let Ok(row) = self.decoded_binding(ctx, op) else {
            return Ok(());
        };
        let Ok(endpoint) = self.endpoint_row(ctx, &row, op).await else {
            return Ok(());
        };
        let target = self.delivery_target(&row, &endpoint);
        self.effects.fence(&target).await.map_err(|error| {
            self.error(EndpointBindingDriverErrorKind::ServingEffect, op).with_detail(
                FailureDetail::at("delete/fence")
                    .comparison(FailureComparison::new(
                        "binding.fence",
                        "established",
                        "the adapter failed",
                    ))
                    .with_note(error),
            )
        })?;
        self.delivered = None;
        Ok(())
    }

    /// Teardown with the preserved drain semantics: the consumer's attachment
    /// is observed BEFORE anything is released, and a consumer that still
    /// holds the exact endpoint keeps the durable deleting mark and its
    /// delivery - a retryable failure - instead of having the endpoint pulled
    /// out from under live use. A detached consumer's delivery is released,
    /// idempotently, so a requeued pass converges.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        // A row whose spec no longer decodes delivered nothing and has
        // nothing to release. The same holds for a row whose exact endpoint
        // is not observable: a delivery is only ever established against a
        // committed endpoint, and this row is the endpoint's child, so the
        // plane retires the endpoint only after this row has retired.
        let Ok(row) = self.decoded_binding(ctx, op) else {
            return Ok(());
        };
        let Ok(endpoint) = self.endpoint_row(ctx, &row, op).await else {
            return Ok(());
        };
        let target = self.delivery_target(&row, &endpoint);
        if self
            .effects
            .consumer_attached(&target)
            .await
            .map_err(|error| {
                self.error(EndpointBindingDriverErrorKind::ServingEffect, op).with_detail(
                    FailureDetail::at("delete/attach")
                        .comparison(FailureComparison::new(
                            "consumer.attached",
                            "released",
                            "the adapter failed",
                        ))
                        .with_note(error),
                )
            })?
        {
            return Err(self
                .error(EndpointBindingDriverErrorKind::DrainPending, op)
                .with_detail(FailureDetail::at("delete/attach").comparison(
                    FailureComparison::new("consumer.attached", "detached", "still attached"),
                )));
        }
        self.effects.release(&target).await.map_err(|error| {
            self.error(EndpointBindingDriverErrorKind::ServingEffect, op).with_detail(
                FailureDetail::at("delete/release")
                    .comparison(FailureComparison::new(
                        "binding.delivery",
                        "released",
                        "the adapter failed",
                    ))
                    .with_note(error),
            )
        })?;
        self.delivered = None;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The `EndpointBinding` type's driver declaration.
///
/// `EndpointBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane
/// cannot serve the converted binding shapes without it, so it must be
/// registered before the plane opens. The type is not exportable:
/// `ResourceExport` admits only qualified `*.d2bus.org.*Service` types, so a
/// binding can never be an export subject. The driver serves no broker
/// operations, contributes no startup steps, and licenses no child creation
/// ([`ENDPOINT_BINDING_CREATIONS`]); the exact `Endpoint` it delivers is
/// declared as a read ([`ENDPOINT_BINDING_READS`]) and the declaration carries
/// the family's declared effects service
/// ([`crate::effects_service::BINDING_EFFECTS_SERVICE`]), which the daemon
/// hosts per zone from the family's registered factory.
pub fn binding_descriptor(args: EndpointBindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::ENDPOINT_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: ENDPOINT_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: ENDPOINT_BINDING_READS,
        operations: &[],
        creations: ENDPOINT_BINDING_CREATIONS,
        startup: &[],
        services: &[BINDING_EFFECTS_SERVICE],
        decoder: binding_spec_decoder(),
        factory: Arc::new(EndpointBindingDriverFactory::new(args)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted serving port and a recording
// manager endpoint with one shared ordered log, so the exact endpoint's
// verification, delivery, and release are asserted as the manager and the
// effect adapter record them.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec;
    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingRealizationFacet, BindingSourceDecision, EndpointAttachmentKind,
        ResourceGeneration, ResourceUid, ZoneRevision, resource_status::StatusCode,
    };
    use d2b_provider_endpoint::endpoint::{
        EndpointAttachmentPolicy, EndpointClass, EndpointLifecyclePolicy, EndpointLocality,
        EndpointTransport, EndpointVisibility,
    };
    use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
    use d2b_resource_runtime::context::ResourceContext;
    use d2b_resource_runtime::driver::{
        DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, FailureKinds};
    use d2b_resource_runtime::identity::{ResourceProvenance, StoredDesiredResource};

    use super::{
        ENDPOINT_DIRECTORY_LISTABLE, ENDPOINT_NOT_READY, EndpointBindingDelivery,
        EndpointBindingDriverArgs, EndpointBindingDriverFactory, EndpointBindingDriverStatus,
        EndpointBindingStatusResource, EndpointSelectorRejection, EndpointSpec, ZoneId,
        binding_spec_decoder, committed_source_selector, exact_endpoint_selector,
    };
    use crate::test_support::FakeEndpointEffects;

    // -- fixtures ------------------------------------------------------------

    /// The exact `Endpoint` row the binding names, in the closed
    /// `EndpointSpec` shape the owner publishes.
    fn endpoint_row_bytes() -> Vec<u8> {
        serde_json::json!({
            "providerRef": "Provider/endpoint",
            "producerRef": "Process/compositor",
            "endpointClass": "service",
            "transport": "unix",
            "purpose": "display",
            "locality": "host-local",
            "visibility": "provider",
            "attachmentPolicy": {"supported": true, "maxAttachments": 2},
            "consumerPolicy": {
                "allowedSubjects": ["Process/consumer"],
                "allowedOperations": ["resolve", "observe", "attach"]
            },
            "lifecyclePolicy": "recycle-with-producer",
        })
        .to_string()
        .into_bytes()
    }

    /// The `Endpoint` row this binding names, owned by nobody and readable
    /// through the manager.
    fn endpoint_row(uid: [u8; 16]) -> StoredDesiredResource {
        StoredDesiredResource {
            key: d2b_resource_runtime::identity::ResourceKey::new("work", "Endpoint", "display"),
            uid,
            generation: 2,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: endpoint_row_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The exact `EndpointBinding` row envelope the plane commits: the
    /// canonical request as the base spec, plus the family's Provider
    /// reference in the envelope layer.
    fn binding_row(owner_uid: [u8; 16]) -> StoredDesiredResource {
        StoredDesiredResource {
            key: d2b_resource_runtime::identity::ResourceKey::new(
                "work",
                super::ENDPOINT_BINDING_TYPE_NAME,
                "display-binding",
            ),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: Some(owner_uid),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: serde_json::json!({
                "providerRef": super::BINDING_PROVIDER_REF,
                "endpointRef": "Endpoint/display",
                "executionRef": "Process/consumer",
                "slot": "primary",
                "attachment": "connect",
                "source": admitted_source(&["consume"], &["endpoint-descriptor"]),
            })
            .to_string()
            .into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The source decision a committed endpoint binding row carries: the
    /// rights the source admitted and the facets it declared.
    fn admitted_source(rights: &[&str], facets: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "admittedRights": rights,
            "arbitration": "shared",
            "realizedFacets": facets,
        })
    }

    /// Rewrite one field of a committed binding row's base spec.
    fn with_spec_field(mut row: StoredDesiredResource, field: &str, value: serde_json::Value) -> StoredDesiredResource {
        let mut spec: serde_json::Value = serde_json::from_slice(&row.spec).expect("binding spec");
        spec[field] = value;
        row.spec = serde_json::to_vec(&spec).expect("binding spec bytes");
        row
    }

    struct Fixture {
        ctx: ResourceContext,
        manager: RecordingManagerEndpoint,
        requeue: RecordingRequeue,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let requeue = RecordingRequeue::default();
        let ctx = ResourceContext::new(
            row,
            binding_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        );
        Fixture {
            ctx,
            manager,
            requeue,
        }
    }

    /// A manager that holds the exact `Endpoint` row this binding names.
    fn manager_with_endpoint() -> RecordingManagerEndpoint {
        RecordingManagerEndpoint::new().with_row(endpoint_row([0x11; 16]))
    }

    async fn driver(effects: Arc<FakeEndpointEffects>) -> Box<dyn DynResourceDriver> {
        EndpointBindingDriverFactory::new(EndpointBindingDriverArgs {
            zone: ZoneId::parse("work").expect("zone"),
            facets: effects.facet_set(),
        })
        .create(&d2b_resource_runtime::identity::ResourceKey::new(
            "work",
            super::ENDPOINT_BINDING_TYPE_NAME,
            "display-binding",
        ))
        .await
    }

    /// The effect-port calls only, with the manager's own watch
    /// registrations filtered out of the shared log.
    ///
    /// The double shares one ordered log with the manager endpoint, so a
    /// manager call and a serving effect read as one sequence; the effect
    /// assertions below are about the effect port alone.
    fn effect_calls(fake: &FakeEndpointEffects) -> Vec<String> {
        fake.call_order()
            .into_iter()
            .filter(|entry| {
                matches!(
                    entry.as_str(),
                    "verify" | "deliver" | "fence" | "attached" | "release"
                )
            })
            .collect()
    }

    /// The status projection the concluding pass published, read back as the
    /// typed wire contract.
    fn published_projection(f: &mut Fixture) -> EndpointBindingStatusResource {
        let projection = f
            .ctx
            .take_status_projection()
            .expect("the concluding pass publishes a projection");
        serde_json::from_value(projection).expect("the projection is the typed wire contract")
    }

    // -- validate: the exact endpoint the row names --------------------------

    /// A binding whose named endpoint does not exist yet defers retryably:
    /// the row may simply not be committed (issue #511), and refusing it
    /// terminal would fail a perfectly good relationship.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_defers_when_the_named_endpoint_does_not_exist() {
        let manager = RecordingManagerEndpoint::new();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager.clone());
        let mut d = driver(fake.clone()).await;

        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the named endpoint does not exist");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
        assert!(
            fake.call_order().is_empty(),
            "nothing is verified or delivered before the endpoint is observable: {:?}",
            fake.call_order()
        );
    }

    /// The same deferral when the manager cannot answer at all: an
    /// unanswerable plane is never reported as absence.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_defers_when_the_manager_cannot_answer() {
        let manager = manager_with_endpoint();
        manager.set_fail_reads(true);
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager.clone());
        let mut d = driver(fake).await;

        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the manager could not answer");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    }

    /// A present endpoint whose stored spec does not decode is terminal, and
    /// a present row whose owner actually differs from this row's owner is
    /// terminal too: neither converges by retrying.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_is_terminal_for_an_undecodable_endpoint_and_an_owner_mismatch() {
        let manager = RecordingManagerEndpoint::new().with_row(StoredDesiredResource {
            spec: b"not an endpoint spec".to_vec(),
            ..endpoint_row([0x11; 16])
        });
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager.clone());
        let mut d = driver(fake).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("undecodable endpoint");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_SPEC_INVALID);

        // A different owner uid on the row the spec names stays terminal.
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x42; 16]), manager);
        let mut d = driver(fake).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the endpoint row belongs to another owner");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_OWNER_MISMATCH);
    }

    /// The exact-endpoint selector rule: a wildcard, an alternation, a
    /// cross-Zone target, a second separator, and the wrong source type are
    /// each refused before the row is admitted, and none of them reaches the
    /// manager or the effect port.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_wildcard_ambiguous_or_cross_zone_selector() {
        for (selector, code) in [
            ("Endpoint/*", "endpoint-selector-wildcard"),
            ("Endpoint/dis*", "endpoint-selector-wildcard"),
            ("Endpoint/display,Endpoint/other", "endpoint-selector-wildcard"),
            ("other-zone:Endpoint/display", "endpoint-selector-cross-zone"),
            ("Endpoint/shared/display", "endpoint-selector-cross-zone"),
            ("Device/display", "endpoint-selector-wrong-type"),
        ] {
            let manager = manager_with_endpoint();
            let fake = FakeEndpointEffects::shared(manager.log_handle());
            let row = with_spec_field(
                binding_row([0x11; 16]),
                "endpointRef",
                serde_json::json!(selector),
            );
            let mut f = fixture(row, manager);
            let mut d = driver(fake.clone()).await;

            let failure = d
                .validate(&mut f.ctx)
                .await
                .expect_err("a selector that could name more is refused");
            assert!(
                failure
                    .comparisons()
                    .iter()
                    .any(|comparison| comparison.observed().render() == code),
                "{selector} names the shape it refused ({code}): {:?}",
                failure.comparisons()
            );
            assert_eq!(
                failure.class(),
                FailureClass::Terminal,
                "{selector} must be refused terminally"
            );
            assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
            assert!(
                f.manager.call_order().is_empty(),
                "{selector} must not reach the manager: {:?}",
                f.manager.call_order()
            );
            assert!(
                fake.call_order().is_empty(),
                "{selector} must not reach the effect port: {:?}",
                fake.call_order()
            );
        }
    }

    /// The same rule over the pure selector reader, which names the shape
    /// each refusal is about.
    #[test]
    fn the_selector_reader_names_the_shape_it_refuses() {
        // The field-name reader: a row that carries no selector, or carries
        // one that is not a string, is refused before the rule runs.
        assert_eq!(
            committed_source_selector(&serde_json::json!({})),
            Err(EndpointSelectorRejection::Missing)
        );
        assert_eq!(
            committed_source_selector(&serde_json::json!({"endpointRef": 7})),
            Err(EndpointSelectorRejection::NotAString)
        );
        assert_eq!(
            committed_source_selector(&serde_json::json!({"endpointRef": "Endpoint/display"})),
            Ok("Endpoint/display")
        );

        // The rule itself, which knows nothing about where a row carries the
        // selector and so survives a change of field name untouched.
        for (selector, expected) in [
            ("Endpoint/display", Ok(())),
            ("Endpoint/display*", Err(EndpointSelectorRejection::Wildcard)),
            ("Endpoint/display?", Err(EndpointSelectorRejection::Wildcard)),
            ("Endpoint/display,Endpoint/other", Err(EndpointSelectorRejection::Wildcard)),
            ("z:Endpoint/display", Err(EndpointSelectorRejection::CrossZone)),
            ("Endpoint/shared/display", Err(EndpointSelectorRejection::CrossZone)),
            ("Socket/display", Err(EndpointSelectorRejection::WrongType)),
        ] {
            assert_eq!(exact_endpoint_selector(selector), expected, "{selector}");
        }
    }

    /// The endpoint's own declaration is the admission authority: a
    /// consumer the endpoint does not name, an operation it never declared,
    /// and an attachment its own ceiling refuses are all terminal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_what_the_endpoints_own_declaration_refuses() {
        // The named consumer is not on the endpoint's subject allowlist.
        let manager = RecordingManagerEndpoint::new().with_row(StoredDesiredResource {
            spec: serde_json::json!({
                "providerRef": "Provider/endpoint",
                "producerRef": "Process/compositor",
                "endpointClass": "service",
                "transport": "unix",
                "purpose": "display",
                "locality": "host-local",
                "visibility": "provider",
                "attachmentPolicy": {"supported": true, "maxAttachments": 2},
                "consumerPolicy": {
                    "allowedSubjects": ["Process/other"],
                    "allowedOperations": ["resolve", "observe", "attach"]
                },
                "lifecyclePolicy": "recycle-with-producer",
            })
            .to_string()
            .into_bytes(),
            ..endpoint_row([0x11; 16])
        });
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the endpoint does not admit this consumer");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PLAN_DERIVATION_INVALID);
        assert!(matches!(
            f.ctx.status::<EndpointBindingDriverStatus>(),
            Some(EndpointBindingDriverStatus::Rejected {
                reason: "endpoint-consumer-not-allowed"
            })
        ));

        // An `attach` on an endpoint that does not support attachments.
        let manager = RecordingManagerEndpoint::new().with_row(StoredDesiredResource {
            spec: serde_json::json!({
                "providerRef": "Provider/endpoint",
                "producerRef": "Process/compositor",
                "endpointClass": "service",
                "transport": "unix",
                "purpose": "display",
                "locality": "host-local",
                "visibility": "provider",
                "attachmentPolicy": {"supported": false, "maxAttachments": 0},
                "consumerPolicy": {"allowedOperations": ["attach"]},
                "lifecyclePolicy": "recycle-with-producer",
            })
            .to_string()
            .into_bytes(),
            ..endpoint_row([0x11; 16])
        });
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let row = with_spec_field(
            binding_row([0x11; 16]),
            "attachment",
            serde_json::json!("attach"),
        );
        let row = with_spec_field(
            row,
            "source",
            admitted_source(
                &["consume"],
                &["endpoint-descriptor", "endpoint-pathname"],
            ),
        );
        let mut f = fixture(row, manager);
        let mut d = driver(fake).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the endpoint admits no attachment");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert!(matches!(
            f.ctx.status::<EndpointBindingDriverStatus>(),
            Some(EndpointBindingDriverStatus::Rejected {
                reason: "endpoint-attachment-refused"
            })
        ));
    }

    /// The endpoint's own vocabulary round-trips: every shape the closed
    /// `EndpointSpec` contract admits decodes, so the parent's decode path
    /// is the real contract and not a narrower local copy.
    #[test]
    fn the_named_endpoint_decodes_as_the_closed_endpoint_contract() {
        let spec: EndpointSpec = serde_json::from_slice(&endpoint_row_bytes())
            .expect("the endpoint row decodes");
        assert_eq!(spec.endpoint_class(), EndpointClass::Service);
        assert_eq!(spec.transport(), EndpointTransport::Unix);
        assert_eq!(spec.locality(), EndpointLocality::HostLocal);
        assert_eq!(spec.visibility(), EndpointVisibility::Provider);
        assert_eq!(spec.lifecycle_policy(), EndpointLifecyclePolicy::RecycleWithProducer);
        assert_eq!(
            spec.attachment_policy(),
            &EndpointAttachmentPolicy::new(true, 2).expect("attachment policy"),
        );
        let policy = spec.consumer_policy();
        assert!(
            policy.admits_subject(
                &d2b_contracts_resource::v3::ResourceRef::parse("Process/consumer")
                    .expect("consumer reference")
            ),
            "the declared subject allowlist admits the consumer it names"
        );
        assert!(
            !policy.admits_subject(
                &d2b_contracts_resource::v3::ResourceRef::parse("Process/other")
                    .expect("consumer reference")
            ),
            "and refuses a consumer it does not name"
        );
        assert!(policy.admits_operation(d2b_provider_endpoint::endpoint::EndpointOperation::Resolve));
    }

    // -- reconcile: the descriptor is verified, never assumed -----------------

    /// Reconcile delivers only what the observation proves: the descriptor
    /// is verified, the delivery names the pinned identity, and the fenced
    /// projection reports ready.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_verifies_the_descriptor_before_it_delivers() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;

        assert_eq!(d.reconcile(&mut f.ctx).await.expect("reconcile"), ReconcileOutcome::Satisfied);
        assert_eq!(
            effect_calls(&fake),
            ["verify", "deliver"],
            "the exact endpoint is verified before anything is delivered"
        );
        assert!(matches!(
            f.ctx.status::<EndpointBindingDriverStatus>(),
            Some(EndpointBindingDriverStatus::Delivering {
                issued: true,
                ..
            })
        ));
        let projection = published_projection(&mut f);
        assert!(projection.ready, "the verified endpoint is being delivered");
        assert!(projection.reason.is_none());
        assert_eq!(projection.fence.generation.get(), 1);
        assert_eq!(
            projection.fence.uid,
            ResourceUid::from_bytes(f.ctx.uid()).expect("the row uid is canonical"),
        );

        // A second pass observes the same pinned identity: the delivery is
        // current and is not re-issued.
        d.reconcile(&mut f.ctx).await.expect("reconcile again");
        assert_eq!(
            fake.count_of("deliver"),
            1,
            "an unchanged exact endpoint is delivered once"
        );
        assert_eq!(fake.count_of("verify"), 2, "every pass observes");
    }

    /// The three shortfalls the verification exists for: effective access
    /// short of the admitted right, an ancestor that no longer applies a
    /// traverse bit, and a containing directory the consumer can enumerate.
    /// None of them delivers anything, and each publishes its own code.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_refuses_to_deliver_what_the_observation_does_not_prove() {
        for (script, code) in [
            ("rights", ENDPOINT_NOT_READY),
            ("traverse", ENDPOINT_NOT_READY),
            ("listable", ENDPOINT_DIRECTORY_LISTABLE),
            ("not-accepting", ENDPOINT_NOT_READY),
        ] {
            let manager = manager_with_endpoint();
            let fake = FakeEndpointEffects::shared(manager.log_handle());
            match script {
                // Read is what a connect needs; a write-only entry is short.
                "rights" => fake.set_effective_rights(0o2),
                "traverse" => fake.set_effective_traverse(0o0),
                "listable" => fake.set_parent_listable(true),
                _ => fake.set_accepting(false),
            }
            let row = with_spec_field(
                with_spec_field(
                    binding_row([0x11; 16]),
                    "attachment",
                    serde_json::json!("attach"),
                ),
                "source",
                admitted_source(
                    &["consume"],
                    &["endpoint-descriptor", "endpoint-pathname"],
                ),
            );
            let mut f = fixture(row, manager);
            let mut d = driver(fake.clone()).await;

            d.reconcile(&mut f.ctx).await.expect("reconcile");
            assert_eq!(
                effect_calls(&fake),
                ["verify"],
                "{script} must not deliver: {:?}",
                effect_calls(&fake)
            );
            let projection = published_projection(&mut f);
            assert!(!projection.ready, "{script} is never reported ready");
            assert_eq!(
                projection.reason.as_ref().map(StatusCode::as_str),
                Some(code),
                "{script} carries its own stable code"
            );
            assert!(
                matches!(
                    f.ctx.status::<EndpointBindingDriverStatus>(),
                    Some(EndpointBindingDriverStatus::Unproven { reason, .. }) if *reason == code
                ),
                "{script} keeps its reason in the in-memory status"
            );
            assert_eq!(
                f.requeue.scheduled().len(),
                1,
                "{script} re-checks on the preserved cadence"
            );
        }
    }

    /// An endpoint that does not accept still serves a `connect`: only an
    /// `attach` needs the endpoint to be accepting.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_non_accepting_endpoint_still_serves_a_connect() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_accepting(false);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(effect_calls(&fake), ["verify", "deliver"]);
        assert!(published_projection(&mut f).ready);
    }

    /// A replaced inode invalidates the delivery: the next pass observes a
    /// different identity and re-delivers against the NEW exact endpoint
    /// rather than keeping the one that was there before.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_replaced_inode_invalidates_the_cached_delivery() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;

        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let first = match f.ctx.status::<EndpointBindingDriverStatus>() {
            Some(EndpointBindingDriverStatus::Delivering { socket, .. }) => *socket,
            other => panic!("expected Delivering, got {other:?}"),
        };
        fake.replace_inode();
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let second = match f.ctx.status::<EndpointBindingDriverStatus>() {
            Some(EndpointBindingDriverStatus::Delivering { socket, .. }) => *socket,
            other => panic!("expected Delivering, got {other:?}"),
        };
        assert_ne!(first, second, "the pass pins the new identity");
        assert_eq!(fake.count_of("deliver"), 2, "the replaced delivery is re-issued");
    }

    /// The delivery form follows the row's own attachment kind: a connect
    /// is a descriptor with no destination, and an attach is a private
    /// presentation at the one destination component the relationship owns.
    #[test]
    fn the_delivery_form_follows_the_rows_own_attachment_kind() {
        // The row is built through the contract's own `new()`, so this test
        // exercises the same admission the driver's seam does.
        let row = |attachment| {
            let facets = if attachment == EndpointAttachmentKind::Attach {
                vec![
                    BindingRealizationFacet::EndpointDescriptor,
                    BindingRealizationFacet::EndpointPathname,
                ]
            } else {
                vec![BindingRealizationFacet::EndpointDescriptor]
            };
            EndpointBindingSpec::new(
                d2b_contracts_resource::v3::ResourceRef::parse("Endpoint/display")
                    .expect("endpoint reference"),
                d2b_contracts_resource::v3::ResourceRef::parse("Process/consumer")
                    .expect("consumer reference"),
                attachment,
                d2b_contracts_resource::v3::execution_policy::BoundedToken::parse(
                    "primary".to_owned(),
                )
                .expect("slot"),
                BindingSourceDecision::new(
                    vec![attachment.requested_rights()],
                    BindingArbitration::Shared,
                    facets,
                )
                .expect("source decision"),
            )
            .expect("binding row")
        };

        let connect = EndpointBindingDelivery::for_row(&row(EndpointAttachmentKind::Connect));
        assert!(connect.form().satisfies(EndpointAttachmentKind::Connect));
        assert!(connect.destination().is_none(), "a descriptor names no destination");
        assert_eq!(
            connect.required_facets(),
            &[d2b_contracts_resource::v3::BindingRealizationFacet::EndpointDescriptor]
        );

        let attach = EndpointBindingDelivery::for_row(&row(EndpointAttachmentKind::Attach));
        assert!(attach.form().satisfies(EndpointAttachmentKind::Attach));
        assert!(
            !attach.form().satisfies(EndpointAttachmentKind::Connect),
            "a presentation is not a descriptor delivery"
        );
        assert_eq!(
            attach.destination().map(|token| token.as_str()),
            Some("primary"),
            "the destination is the row's own stable consumer slot"
        );
        assert_eq!(
            attach.required_facets(),
            &[
                d2b_contracts_resource::v3::BindingRealizationFacet::EndpointDescriptor,
                d2b_contracts_resource::v3::BindingRealizationFacet::EndpointPathname,
            ]
        );
    }

    /// The dependency edges are the exact endpoint and the admitted
    /// consumer, registered once each.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn dependency_watches_cover_the_endpoint_and_the_consumer_once() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager.clone());
        let mut d = driver(fake).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile one");
        d.reconcile(&mut f.ctx).await.expect("reconcile two");

        let targets = manager
            .watch_targets()
            .iter()
            .map(|key| format!("{}/{}", key.type_name, key.name))
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            vec!["Endpoint/display".to_owned(), "Process/consumer".to_owned()],
            "one watch per dependency, and no duplicates"
        );
    }

    /// A serving-effect failure is retryable, and nothing is claimed as
    /// delivered when the adapter could not establish the delivery.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failed_serving_effect_defers_retryably() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_fail_deliver(true);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("the delivery failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_fail_verify(true);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake).await;
        let failure = d
            .reconcile(&mut f.ctx)
            .await
            .expect_err("the observation failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
    }

    // -- recover: re-adopt without re-delivering ------------------------------

    /// A restart re-adopts the exact endpoint by VERIFYING it and never
    /// re-delivers: a fresh driver holds no cached identity and issues no
    /// fresh delivery to a consumer that already holds the endpoint.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_re_adopts_the_exact_endpoint_without_re_delivering() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        // Pre-restart: the relationship is delivered and its pinned identity
        // is known.
        let mut f = fixture(binding_row([0x11; 16]), manager.clone());
        let mut d = driver(Arc::clone(&fake)).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let pre = match f.ctx.status::<EndpointBindingDriverStatus>() {
            Some(EndpointBindingDriverStatus::Delivering { socket, .. }) => *socket,
            other => panic!("expected Delivering, got {other:?}"),
        };
        let before = effect_calls(&fake).len();

        // Restart: a fresh context and a fresh driver over the same durable
        // rows.
        let mut f2 = fixture(binding_row([0x11; 16]), manager);
        let mut d2 = driver(Arc::clone(&fake)).await;
        assert_eq!(
            d2.recover(&mut f2.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "the exact endpoint is verified and re-adopted"
        );
        let after = &effect_calls(&fake)[before..];
        assert_eq!(
            after,
            ["verify"],
            "recovery observes the endpoint; it does not deliver it again: {after:?}"
        );
        assert_eq!(fake.count_of("deliver"), 1, "one delivery across the restart");
        match f2.ctx.status::<EndpointBindingDriverStatus>() {
            Some(EndpointBindingDriverStatus::Recovered { socket, .. }) => {
                assert_eq!(*socket, pre, "the same exact endpoint is re-adopted");
            }
            other => panic!("expected Recovered, got {other:?}"),
        }
    }

    /// Recovery adopts nothing when the exact endpoint is not there, and
    /// nothing when the observation no longer proves it: both wait for the
    /// next reconcile pass rather than claiming an adoption.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_adopts_nothing_without_a_provable_exact_endpoint() {
        // The endpoint row is gone.
        let manager = RecordingManagerEndpoint::new();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        assert_eq!(d.recover(&mut f.ctx).await.expect("recover"), RecoveryOutcome::Missing);
        assert!(fake.call_order().is_empty(), "an absent endpoint is never verified");

        // The endpoint is there but its containing directory is listable.
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_parent_listable(true);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        assert_eq!(d.recover(&mut f.ctx).await.expect("recover"), RecoveryOutcome::Missing);
        assert_eq!(fake.call_order(), ["verify"]);
        assert!(
            fake.call_order().iter().all(|entry| entry.as_str() != "deliver"),
            "an unproven endpoint is never delivered on recovery"
        );
    }

    // -- teardown -------------------------------------------------------------

    /// The teardown fences new use, observes the consumer's attachment
    /// BEFORE anything is released, and releases only a detached consumer's
    /// delivery.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn pre_drain_fences_the_relationship_and_teardown_releases_it() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let before = effect_calls(&fake).len();

        d.pre_drain(&mut f.ctx).await.expect("pre-drain");
        d.delete(&mut f.ctx).await.expect("delete");
        let after = effect_calls(&fake)[before..].to_vec();
        assert_eq!(
            after,
            ["fence", "attached", "release"],
            "new use is fenced first, the attachment is observed before the \
             release, and only then is the delivery released"
        );
    }

    /// A consumer that still holds the exact endpoint blocks the teardown:
    /// the durable deleting mark and the delivery both stay, and the pass is
    /// retryable rather than pulling the endpoint out from under live use.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_attached_consumer_blocks_the_teardown() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        fake.set_attached(true);

        let failure = d
            .delete(&mut f.ctx)
            .await
            .expect_err("the consumer still holds the endpoint");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::CHILDREN_DRAINING);
        assert!(
            fake.call_order().iter().all(|entry| entry.as_str() != "release"),
            "nothing is released while the consumer is attached: {:?}",
            fake.call_order()
        );

        // The consumer detached: the same pass converges, idempotently.
        fake.set_attached(false);
        d.delete(&mut f.ctx).await.expect("delete after detach");
        d.delete(&mut f.ctx).await.expect("a requeued pass converges");
        assert_eq!(fake.count_of("release"), 2);
    }

    /// A release the adapter cannot complete defers retryably, and a row
    /// whose attachment cannot be observed is never treated as detached.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failed_release_or_attachment_observation_defers_retryably() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_fail_release(true);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        let failure = d.delete(&mut f.ctx).await.expect_err("the release failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_fail_attached(true);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        let failure = d
            .delete(&mut f.ctx)
            .await
            .expect_err("the attachment could not be observed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert!(fake.call_order().iter().all(|entry| entry.as_str() != "release"));
    }

    /// A row that never decoded delivered nothing, so its teardown converges
    /// without touching the effect port.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_row_that_never_decoded_tears_down_without_an_effect() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut row = binding_row([0x11; 16]);
        row.spec = b"not a binding envelope".to_vec();
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;

        d.pre_drain(&mut f.ctx).await.expect("pre-drain");
        d.delete(&mut f.ctx).await.expect("delete");
        assert!(
            fake.call_order().is_empty(),
            "an undecodable row released nothing: {:?}",
            fake.call_order()
        );
    }

    /// The row contract's own admission runs inside this crate's seam: a
    /// consumer the binding kind does not admit - a `Host`, for an endpoint -
    /// and a source that is not an `Endpoint` are both refused terminally
    /// before the row reaches the manager, and a consumer the kind DOES
    /// admit, a helper `Process`, is served. The consumer set is the
    /// contract's predicate, not a bound this crate asserts.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_row_contract_admits_a_helper_process_and_refuses_a_host() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        d.validate(&mut f.ctx).await.expect("a helper Process is admitted");

        // A Host is never the consumer of an endpoint binding.
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let row = with_spec_field(
            binding_row([0x11; 16]),
            "executionRef",
            serde_json::json!("Host/host-system"),
        );
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("a Host consumer is not admitted by the row contract");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
        assert!(
            f.manager.call_order().is_empty(),
            "a row the contract refused never reaches the manager: {:?}",
            f.manager.call_order()
        );

        // A source that is not an Endpoint is refused by the same rule.
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let row = with_spec_field(
            binding_row([0x11; 16]),
            "endpointRef",
            serde_json::json!("Device/gpu"),
        );
        let mut f = fixture(row, manager);
        let mut d = driver(fake).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("a non-Endpoint source is not admitted by the row contract");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
    }

    /// The row's committed source decision is authority, not bookkeeping: a
    /// row whose right the source never admitted is refused terminally by
    /// every verb that reads the row, and NOTHING reaches the effect port -
    /// which is what makes the check non-inert. `validate`, `reconcile`, and
    /// `recover` are all driven, because a check that ran only on the first
    /// attach would let a later verb act on a decision it never verified.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_right_the_source_never_admitted_is_refused_by_every_verb() {
        // A `listen` row claims the observing right; this decision admits
        // only the consuming one, so the source never granted it.
        let row = with_spec_field(
            with_spec_field(
                binding_row([0x11; 16]),
                "attachment",
                serde_json::json!("listen"),
            ),
            "source",
            admitted_source(&["consume"], &["endpoint-descriptor"]),
        );

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(row.clone(), manager.clone());
        let mut d = driver(fake.clone()).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the source never admitted this right");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
        assert!(
            fake.call_order().is_empty(),
            "a withheld right reaches no effect: {:?}",
            fake.call_order()
        );

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(row.clone(), manager);
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx)
            .await
            .expect_err("reconcile refuses it too");
        assert!(fake.call_order().is_empty(), "{:?}", fake.call_order());

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;
        d.recover(&mut f.ctx)
            .await
            .expect_err("recover refuses it too");
        assert!(fake.call_order().is_empty(), "{:?}", fake.call_order());
    }

    /// The same for the facet: a source decision that declares only the
    /// endpoint descriptor does not admit an `attach`, whose delivery
    /// addresses the socket by name and needs the private pathname too.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_facet_the_source_never_declared_is_refused_by_every_verb() {
        let row = with_spec_field(
            with_spec_field(
                binding_row([0x11; 16]),
                "attachment",
                serde_json::json!("attach"),
            ),
            "source",
            admitted_source(&["consume"], &["endpoint-descriptor"]),
        );

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(row.clone(), manager.clone());
        let mut d = driver(fake.clone()).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("the source never declared the pathname facet");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
        assert!(
            fake.call_order().is_empty(),
            "an undeclared facet reaches no effect: {:?}",
            fake.call_order()
        );

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(row.clone(), manager);
        let mut d = driver(fake.clone()).await;
        d.reconcile(&mut f.ctx)
            .await
            .expect_err("reconcile refuses it too");
        assert!(fake.call_order().is_empty(), "{:?}", fake.call_order());

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;
        d.recover(&mut f.ctx)
            .await
            .expect_err("recover refuses it too");
        assert!(fake.call_order().is_empty(), "{:?}", fake.call_order());
    }

    /// The two authority checks are independent, and a row the source DID
    /// admit is served: this implementation's own declared support answers a
    /// different question from the source decision, and a row passes only by
    /// satisfying both.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_row_the_source_admitted_is_delivered() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake.clone()).await;
        d.validate(&mut f.ctx).await.expect("validate");
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        assert_eq!(effect_calls(&fake), ["verify", "deliver"]);
        assert!(published_projection(&mut f).ready);
    }

    // -- spec refusal ---------------------------------------------------------

    /// The terminal refusal surface at the decode: an envelope that does not
    /// decode, and a `providerRef` naming a Provider outside this family.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn decoded_binding_refuses_an_undecodable_spec_and_an_unsupported_provider() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut row = binding_row([0x11; 16]);
        row.spec = b"not a binding envelope".to_vec();
        let mut f = fixture(row, manager);
        let mut d = driver(fake.clone()).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("undecodable spec");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);

        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let row = with_spec_field(
            binding_row([0x11; 16]),
            "providerRef",
            serde_json::json!("Provider/endpoint-other"),
        );
        let mut f = fixture(row, manager);
        let mut d = driver(fake).await;
        let failure = d
            .validate(&mut f.ctx)
            .await
            .expect_err("unsupported provider");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PROVIDER_UNSUPPORTED);
    }

    /// A row that names no Provider at all is served by the declaring
    /// Provider, which is this type's own driver.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_row_without_a_provider_reference_is_served_by_the_declaring_provider() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        let mut row = binding_row([0x11; 16]);
        let mut spec: serde_json::Value = serde_json::from_slice(&row.spec).expect("binding spec");
        spec.as_object_mut().expect("spec object").remove("providerRef");
        row.spec = serde_json::to_vec(&spec).expect("binding spec bytes");
        let mut f = fixture(row, manager);
        let mut d = driver(fake).await;
        d.validate(&mut f.ctx).await.expect("validate");
    }

    // -- the fenced projection ------------------------------------------------

    /// The fence cannot be satisfied vacuously: a projection authored under
    /// another identity, under another generation, or ahead of the stored
    /// revision never reports the row ready.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_stale_or_unproven_fence_never_reports_ready() {
        let manager = manager_with_endpoint();
        let fake = FakeEndpointEffects::shared(manager.log_handle());
        fake.set_effective_rights(0o0);
        let mut f = fixture(binding_row([0x11; 16]), manager);
        let mut d = driver(fake).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let pending = published_projection(&mut f);
        assert!(!pending.ready, "no effective access was proven");
        assert_eq!(
            pending.reason.as_ref().map(StatusCode::as_str),
            Some(ENDPOINT_NOT_READY)
        );

        let uid = pending.fence.uid.clone();
        let generation = ResourceGeneration::new(1).expect("generation");
        assert!(!pending.readiness_is_current(&uid, generation, ZoneRevision::new(1)));
        assert!(
            !pending.readiness_is_current(
                &ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("foreign uid"),
                generation,
                ZoneRevision::new(1),
            ),
            "a foreign identity never reports the row ready"
        );
        assert!(
            !pending.readiness_is_current(
                &uid,
                ResourceGeneration::new(2).expect("generation"),
                ZoneRevision::new(1),
            ),
            "a fence from another generation never reports the row ready"
        );
    }
}
