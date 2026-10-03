//! The clipboard Provider's declared service methods and its host endpoint
//! authority vocabulary (U28).
//!
//! Two authorities used to live beside the graph: a hand-written catalog of
//! service-package strings that decided which role an authenticated route
//! served, and the host channel a host or Guest selection was delivered
//! through. Both are declared here instead.
//!
//! [`CLIPBOARD_SERVICES`] is the Provider's one service-method source. The
//! session admission resolves a route's role through it rather than
//! re-matching package strings, so a method cannot be served by a service that
//! does not declare it.
//!
//! [`clipboard_endpoint_bindings`] is the Provider's one endpoint-grant
//! source. Every host connection is a typed [`EndpointBindingRequest`] over
//! the exact `Endpoint` row the caller declared, in one stable consumer slot,
//! for one bounded purpose; [`admit_clipboard_endpoint`] is the only gate that
//! admits one, and it re-derives the relationship rather than trusting an
//! observed request.
//!
//! Nothing here reads clipboard content. A MIME token, a payload byte, or a
//! history token can never become a slot, a purpose, a consumer, or a source
//! endpoint, because no content value is an input to any derivation in this
//! module and the slot and purpose vocabularies are closed lower-kebab tokens
//! that a payload cannot parse as.

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingKey, BindingSlot, DesiredRevision, EndpointAttachmentKind,
    EndpointBindingRequest, RefusalReason, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, execution_policy::BoundedToken,
    identity::ReconnectGeneration,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use super::ClipboardServiceRole;

/// The display family's declared service package, through which a local
/// desktop `User` reaches the clipboard Provider.
///
/// The identifier is the display Provider's own declaration (U26); the
/// clipboard Provider consumes it as a declared dependency and never mints or
/// mutates it.
pub const DISPLAY_DESKTOP_SERVICE: &str = "d2b.display.v3";

/// The display family's declared Provider reference, the authority behind
/// [`DISPLAY_DESKTOP_SERVICE`]. It is the crate's one display-family Provider
/// reference ([`crate::DISPLAY_PROVIDER_REF`]), consumed here as a declared
/// dependency and never minted or mutated by the clipboard Provider.
pub const DISPLAY_DESKTOP_PROVIDER_REF: &str = crate::DISPLAY_PROVIDER_REF;

/// The declared methods of the clipboard management service.
///
/// The management service answers the Provider's own reconciliation verbs: the
/// display dependency reconcile and the bounded audit flush. It carries no
/// selection carriage, so neither of its methods can move clipboard content.
const MANAGEMENT_METHODS: &[ServiceMethod] = &[
    ServiceMethod::zone_plane("reconcile-display-dependency"),
    ServiceMethod::zone_plane("flush-audit"),
];

/// The declared methods of the clipboard bridge service.
///
/// The bridge service is the only service that carries selection bytes, and
/// each of its methods declares the direction it moves content in: the two
/// capture methods move bytes toward the Provider, the paste methods move
/// bytes toward a Guest, and `guest-selection-event` moves no bytes at all -
/// it mints the opaque echo-suppression receipt that names a history digest.
const BRIDGE_METHODS: &[ServiceMethod] = &[
    ServiceMethod::zone_plane("capture-guest-selection"),
    ServiceMethod::zone_plane("capture-host-selection"),
    ServiceMethod::zone_plane("guest-selection-event"),
    ServiceMethod::zone_plane("authorize-paste"),
    ServiceMethod::zone_plane("materialize-paste"),
];

/// The declared methods of the picker coordination service.
///
/// The picker service carries no content either: `complete-picker` consumes a
/// picker receipt and a result, and the payload is materialized separately
/// through the bridge service's `materialize-paste`.
const PICKER_METHODS: &[ServiceMethod] =
    &[ServiceMethod::zone_plane("complete-picker")];

/// The clipboard Provider's declared management service.
pub const CLIPBOARD_MANAGEMENT_SERVICE: ServiceDecl = ServiceDecl {
    id: crate::MANAGEMENT_SERVICE,
    methods: MANAGEMENT_METHODS,
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The clipboard Provider's declared bridge service.
pub const CLIPBOARD_BRIDGE_SERVICE: ServiceDecl = ServiceDecl {
    id: crate::BRIDGE_SERVICE,
    methods: BRIDGE_METHODS,
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The clipboard Provider's declared picker coordination service.
pub const CLIPBOARD_PICKER_SERVICE: ServiceDecl = ServiceDecl {
    id: crate::PICKER_SERVICE,
    methods: PICKER_METHODS,
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// One declared clipboard service: its role plus the declaration that serves
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredClipboardService {
    /// The session role this service admits.
    pub role: ClipboardServiceRole,
    /// The service's declaration, including its declared methods.
    pub declaration: &'static ServiceDecl,
}

/// Every service the clipboard Provider declares, in role order.
///
/// This is the Provider's single service-method source: the session admission,
/// the controller's runner contract, and the composed host all resolve a
/// service through this table, so a method name exists in exactly one place.
pub const CLIPBOARD_SERVICES: &[DeclaredClipboardService] = &[
    DeclaredClipboardService {
        role: ClipboardServiceRole::Management,
        declaration: &CLIPBOARD_MANAGEMENT_SERVICE,
    },
    DeclaredClipboardService {
        role: ClipboardServiceRole::Bridge,
        declaration: &CLIPBOARD_BRIDGE_SERVICE,
    },
    DeclaredClipboardService {
        role: ClipboardServiceRole::Picker,
        declaration: &CLIPBOARD_PICKER_SERVICE,
    },
];

/// Resolve one authenticated service package to the role that serves it.
///
/// A package this Provider does not declare resolves to `None`, so an
/// unrelated service's route can never inherit a clipboard role.
pub fn clipboard_service_role(service_id: &str) -> Option<ClipboardServiceRole> {
    CLIPBOARD_SERVICES
        .iter()
        .find(|declared| declared.declaration.id == service_id)
        .map(|declared| declared.role)
}

/// Borrow the declaration of one declared service role.
pub fn clipboard_service_declaration(
    role: ClipboardServiceRole,
) -> &'static ServiceDecl {
    CLIPBOARD_SERVICES
        .iter()
        .find(|declared| declared.role == role)
        .map_or(&CLIPBOARD_MANAGEMENT_SERVICE, |declared| {
            declared.declaration
        })
}

/// Whether one declared service answers one declared method name.
pub fn clipboard_service_declares(role: ClipboardServiceRole, method: &str) -> bool {
    clipboard_service_declaration(role).declares_method(method)
}

/// One declared clipboard delivery channel and the endpoint relationship it
/// requires.
///
/// The three roles are the Provider's existing attachment classes: the Guest
/// transfer channel and the two directions of a host selection. Each carries
/// its own stable consumer slot and its own bounded purpose, so no two
/// channels can share one relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClipboardEndpointRole {
    /// Guest clipboard transfer into and out of the Provider.
    GuestTransfer,
    /// The Provider reading a selection the desktop session owns.
    HostSelectionRead,
    /// The Provider supplying a selection into the desktop session.
    HostSelectionSupply,
}

impl ClipboardEndpointRole {
    /// Every delivery role, in the Provider's preserved channel order.
    pub const ALL: [Self; 3] = [
        Self::GuestTransfer,
        Self::HostSelectionRead,
        Self::HostSelectionSupply,
    ];

    /// The stable consumer slot this channel is admitted under.
    pub const fn slot(self) -> &'static str {
        match self {
            Self::GuestTransfer => "clipboard-guest-transfer",
            Self::HostSelectionRead => "clipboard-host-selection-read",
            Self::HostSelectionSupply => "clipboard-host-selection-supply",
        }
    }

    /// The bounded usage purpose this channel is admitted for.
    pub const fn purpose(self) -> &'static str {
        match self {
            Self::GuestTransfer => "clipboard-guest-transfer",
            Self::HostSelectionRead => "clipboard-host-selection-read",
            Self::HostSelectionSupply => "clipboard-host-selection-supply",
        }
    }

    /// The attachment form the consumer uses to reach this channel.
    pub const fn attachment(self) -> EndpointAttachmentKind {
        EndpointAttachmentKind::Connect
    }
}

/// The exact `Endpoint` rows the clipboard Provider declares for its three
/// delivery channels.
///
/// The references are row identities, not locators: the endpoint's own socket
/// stays private to its realization, and the composing side reads them from
/// the committed graph rather than from a name or an environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHostEndpoints {
    guest_transfer: ResourceRef,
    host_selection_read: ResourceRef,
    host_selection_supply: ResourceRef,
}

impl ClipboardHostEndpoints {
    /// Bind one declared `Endpoint` row to each delivery channel.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardEndpointError::SourceNotEndpoint`] when any reference
    /// is not an `Endpoint` row. Two channels may name the same endpoint only
    /// when they also carry different slots and purposes, which the binding
    /// derivation enforces.
    pub fn new(
        guest_transfer: ResourceRef,
        host_selection_read: ResourceRef,
        host_selection_supply: ResourceRef,
    ) -> Result<Self, ClipboardEndpointError> {
        for source in [
            &guest_transfer,
            &host_selection_read,
            &host_selection_supply,
        ] {
            if source.resource_type().as_str() != "Endpoint" {
                return Err(ClipboardEndpointError::SourceNotEndpoint);
            }
        }
        Ok(Self {
            guest_transfer,
            host_selection_read,
            host_selection_supply,
        })
    }

    /// Borrow the declared `Endpoint` row of one channel.
    pub fn source_ref(&self, role: ClipboardEndpointRole) -> &ResourceRef {
        match role {
            ClipboardEndpointRole::GuestTransfer => &self.guest_transfer,
            ClipboardEndpointRole::HostSelectionRead => &self.host_selection_read,
            ClipboardEndpointRole::HostSelectionSupply => &self.host_selection_supply,
        }
    }
}

/// One derived clipboard endpoint relationship.
///
/// The relationship is derived from the Provider's declared channel vocabulary
/// and the consumer row's identity, so a consumer can only ever reach the
/// exact endpoint this value names, in the declared attachment form, for the
/// declared purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEndpointBinding {
    role: ClipboardEndpointRole,
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    request: EndpointBindingRequest,
}

impl ClipboardEndpointBinding {
    /// Return the delivery channel this relationship belongs to.
    pub const fn role(&self) -> ClipboardEndpointRole {
        self.role
    }

    /// Borrow the exact source `Endpoint`.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the exact consumer row.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the typed binding request.
    pub const fn request(&self) -> &EndpointBindingRequest {
        &self.request
    }
}

/// Every endpoint relationship one clipboard consumer row requires, in the
/// Provider's preserved channel order.
///
/// # Errors
///
/// Returns [`ClipboardEndpointError::RequestRejected`] when a declared slot or
/// purpose cannot be expressed as a bounded token, or when the consumer row
/// is not a kind an `EndpointBinding` admits.
pub fn clipboard_endpoint_bindings(
    endpoints: &ClipboardHostEndpoints,
    consumer: &ResourceRef,
) -> Result<Vec<ClipboardEndpointBinding>, ClipboardEndpointError> {
    let mut bindings = Vec::with_capacity(ClipboardEndpointRole::ALL.len());
    for role in ClipboardEndpointRole::ALL {
        let source_ref = endpoints.source_ref(role).clone();
        let slot = BindingSlot::parse(role.slot())
            .map_err(|_| ClipboardEndpointError::RequestRejected)?;
        let purpose = BoundedToken::parse(role.purpose())
            .map_err(|_| ClipboardEndpointError::RequestRejected)?;
        let request = EndpointBindingRequest::new(
            source_ref.clone(),
            consumer.clone(),
            slot,
            role.attachment(),
            purpose,
        )
        .map_err(|_| ClipboardEndpointError::RequestRejected)?;
        bindings.push(ClipboardEndpointBinding {
            role,
            source_ref,
            consumer_ref: consumer.clone(),
            request,
        });
    }
    Ok(bindings)
}

/// The lifecycle one admitted clipboard endpoint relationship is in.
///
/// A relationship starts [`Self::Admitted`] and reaches every other phase only
/// through a transition, so a revocation cannot be expressed as a
/// construction and delivery cannot begin in a revoked state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEndpointPhase {
    /// The relationship is admitted and may open new deliveries.
    Admitted,
    /// Outstanding use is being driven to its safe state.
    Draining,
    /// The relationship is revoked; nothing new may be delivered.
    Revoked,
}

/// The committed evidence one admitted clipboard endpoint relationship is
/// fenced against.
///
/// Advancing any field is the graph saying "older evidence no longer decides
/// this relationship", so a delivery attempt carrying the previous value
/// refuses at the matching stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEndpointFence {
    zone: ZoneId,
    store: StoreIncarnation,
    desired_revision: DesiredRevision,
    sequence: ZoneDesiredSequence,
    source_generation: ResourceGeneration,
    consumer_generation: ResourceGeneration,
    minimum_reconnect: ReconnectGeneration,
    phase: ClipboardEndpointPhase,
}

impl ClipboardEndpointFence {
    /// Construct the fence of a freshly admitted relationship.
    pub const fn new(
        zone: ZoneId,
        store: StoreIncarnation,
        desired_revision: DesiredRevision,
        sequence: ZoneDesiredSequence,
        source_generation: ResourceGeneration,
        consumer_generation: ResourceGeneration,
        minimum_reconnect: ReconnectGeneration,
    ) -> Self {
        Self {
            zone,
            store,
            desired_revision,
            sequence,
            source_generation,
            consumer_generation,
            minimum_reconnect,
            phase: ClipboardEndpointPhase::Admitted,
        }
    }

    /// Borrow the Zone this relationship belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store incarnation the relationship was admitted in.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// Return the committed desired revision the relationship is fenced at.
    pub const fn desired_revision(&self) -> DesiredRevision {
        self.desired_revision
    }

    /// Return the Zone desired sequence the relationship is fenced at.
    pub const fn sequence(&self) -> ZoneDesiredSequence {
        self.sequence
    }

    /// Return the source generation the relationship is fenced at.
    pub const fn source_generation(&self) -> ResourceGeneration {
        self.source_generation
    }

    /// Return the consumer generation the relationship is fenced at.
    pub const fn consumer_generation(&self) -> ResourceGeneration {
        self.consumer_generation
    }

    /// Return the lowest reconnect generation this relationship still admits.
    pub const fn minimum_reconnect(&self) -> ReconnectGeneration {
        self.minimum_reconnect
    }

    /// Return the lifecycle phase the relationship is in.
    pub const fn phase(&self) -> ClipboardEndpointPhase {
        self.phase
    }

    /// Fence the relationship at a newer committed desired revision.
    #[must_use]
    pub fn advance_desired_revision(mut self, desired_revision: DesiredRevision) -> Self {
        self.desired_revision = desired_revision;
        self
    }

    /// Fence the relationship at a newer Zone desired sequence.
    #[must_use]
    pub fn advance_sequence(mut self, sequence: ZoneDesiredSequence) -> Self {
        self.sequence = sequence;
        self
    }

    /// Raise the lowest reconnect generation this relationship admits.
    #[must_use]
    pub fn raise_minimum_reconnect(
        mut self,
        minimum_reconnect: ReconnectGeneration,
    ) -> Self {
        self.minimum_reconnect = minimum_reconnect;
        self
    }

    /// Move the relationship to the draining phase.
    #[must_use]
    pub fn drain(mut self) -> Self {
        self.phase = ClipboardEndpointPhase::Draining;
        self
    }

    /// Move the relationship to the revoked phase.
    #[must_use]
    pub fn revoke(mut self) -> Self {
        self.phase = ClipboardEndpointPhase::Revoked;
        self
    }
}

/// Live, non-secret observations one delivery attempt presents.
///
/// The evidence is measured against the fence and never merges with it: a
/// caller cannot move a fence by presenting evidence, and a fence cannot
/// adopt a caller's claims without the graph having committed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEndpointEvidence {
    /// The Zone the attempt claims to be operating in.
    pub zone: ZoneId,
    /// The store incarnation the attempt observed.
    pub store: StoreIncarnation,
    /// The source generation the attempt observed.
    pub source_generation: ResourceGeneration,
    /// The consumer generation the attempt observed.
    pub consumer_generation: ResourceGeneration,
    /// The desired revision the attempt observed.
    pub desired_revision: DesiredRevision,
    /// The Zone desired sequence the attempt observed.
    pub sequence: ZoneDesiredSequence,
    /// The reconnect generation this attempt is.
    pub reconnect: ReconnectGeneration,
}

/// One admitted clipboard endpoint relationship.
///
/// The admission is the exact identity of the relationship, the fence it was
/// proved against, and the channel it belongs to: later evidence cannot reuse
/// it for a different endpoint, consumer, purpose, or generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedClipboardEndpoint {
    binding: ClipboardEndpointBinding,
    key: BindingKey,
    fence: ClipboardEndpointFence,
}

impl AdmittedClipboardEndpoint {
    /// Return the admitted delivery channel.
    pub const fn role(&self) -> ClipboardEndpointRole {
        self.binding.role
    }

    /// Borrow the admitted relationship.
    pub const fn binding(&self) -> &ClipboardEndpointBinding {
        &self.binding
    }

    /// Borrow the KTD3 identity of the admitted relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the fence this admission was proved against.
    pub const fn fence(&self) -> &ClipboardEndpointFence {
        &self.fence
    }

    /// Whether this admission still admits new delivery.
    pub fn admits_delivery(&self) -> bool {
        self.fence.phase() == ClipboardEndpointPhase::Admitted
    }
}

/// One retained clipboard endpoint grant: an admitted relationship plus the
/// fence the graph has most recently committed for it.
///
/// The two are separate on purpose. Admission proves the relationship once;
/// the retained fence is what the composing host re-fences when the graph
/// moves on, so a revoked or draining relationship stops delivering without
/// the composing side having to re-run admission or drop the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEndpointGrant {
    admitted: AdmittedClipboardEndpoint,
    fence: ClipboardEndpointFence,
}

impl ClipboardEndpointGrant {
    /// Retain one admitted relationship at the fence it was admitted against.
    pub fn new(admitted: AdmittedClipboardEndpoint) -> Self {
        let fence = admitted.fence().clone();
        Self { admitted, fence }
    }

    /// Return the admitted delivery channel.
    pub const fn role(&self) -> ClipboardEndpointRole {
        self.admitted.role()
    }

    /// Borrow the admitted relationship.
    pub const fn admitted(&self) -> &AdmittedClipboardEndpoint {
        &self.admitted
    }

    /// Borrow the fence the graph has most recently committed.
    pub const fn fence(&self) -> &ClipboardEndpointFence {
        &self.fence
    }

    /// Re-fence the retained relationship at newer committed state.
    ///
    /// The relationship identity does not change: only the evidence the
    /// graph commits for it moves, and the next delivery is measured against
    /// the newer fence.
    #[must_use]
    pub fn refenced(mut self, fence: ClipboardEndpointFence) -> Self {
        self.fence = fence;
        self
    }

    /// Why this grant can no longer carry a delivery, when it cannot.
    ///
    /// `None` means the relationship still admits delivery. A withdrawn or
    /// superseded fence answers the refusal that stops it, and the caller
    /// refuses at that point instead of looking for another host channel.
    pub fn delivery_refusal(&self) -> Option<ClipboardEndpointRefusal> {
        if self.fence.phase() == ClipboardEndpointPhase::Admitted
            && self.fence == *self.admitted.fence()
        {
            None
        } else if self.fence.phase() == ClipboardEndpointPhase::Revoked {
            Some(ClipboardEndpointRefusal::relationship_revoked())
        } else if self.fence.phase() == ClipboardEndpointPhase::Draining {
            Some(ClipboardEndpointRefusal::relationship_draining())
        } else {
            Some(ClipboardEndpointRefusal::relationship_superseded())
        }
    }
 }

/// One closed, field-free refusal from the clipboard endpoint gate.
///
/// The stage and reason are the graph's own vocabulary and the code is a
/// stable label: a refusal never echoes a zone, a payload, a socket, or any
/// caller-supplied text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardEndpointRefusal {
    stage: AdmissionStage,
    reason: RefusalReason,
    code: &'static str,
}

impl ClipboardEndpointRefusal {
    const fn at(stage: AdmissionStage, reason: RefusalReason, code: &'static str) -> Self {
        Self { stage, reason, code }
    }

    /// The refusal for a delivery channel that carries no admitted
    /// relationship at all.
    ///
    /// This is the fail-closed answer for a missing endpoint: the channel is
    /// refused at the activate stage as absent authority, which is what stops
    /// a delivery from reaching for another host channel instead.
    pub const fn relationship_absent() -> Self {
        Self::at(
            AdmissionStage::Activate,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-relationship-absent",
        )
    }

    /// The refusal for a relationship the graph has revoked.
    pub const fn relationship_revoked() -> Self {
        Self::at(
            AdmissionStage::Revoke,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-relationship-revoked",
        )
    }

    /// The refusal for a relationship the graph is draining.
    pub const fn relationship_draining() -> Self {
        Self::at(
            AdmissionStage::Drain,
            RefusalReason::UnprovenEffect,
            "clipboard-endpoint-relationship-draining",
        )
    }

    /// The refusal for a relationship whose committed fence has moved past the
    /// one it was admitted against.
    pub const fn relationship_superseded() -> Self {
        Self::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-relationship-superseded",
        )
    }

    /// The refusal for a relationship the graph revoked or began draining,
    /// whichever phase it reached first.
    pub const fn relationship_withdrawn() -> Self {
        Self::relationship_revoked()
    }

    /// The refusal for a relationship presented under the wrong channel.
    pub const fn channel_mismatch() -> Self {
        Self::at(
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
            "clipboard-endpoint-channel-mismatch",
        )
    }

    /// Return the enforcing stage.
    pub const fn stage(self) -> AdmissionStage {
        self.stage
    }

    /// Return the typed refusal reason.
    pub const fn reason(self) -> RefusalReason {
        self.reason
    }

    /// Return the closed, stable refusal code.
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl core::fmt::Display for ClipboardEndpointRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code)
    }
}

impl std::error::Error for ClipboardEndpointRefusal {}

/// Clipboard endpoint derivation and admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEndpointError {
    /// A declared delivery channel did not name an `Endpoint` row.
    SourceNotEndpoint,
    /// The declared relationship could not be expressed as a typed request.
    RequestRejected,
    /// The observed delivery attempt was refused by the graph.
    Refused(ClipboardEndpointRefusal),
}

impl ClipboardEndpointRefusal {
    /// The refusal as the crate's closed endpoint error.
    pub const fn as_error(self) -> ClipboardEndpointError {
        ClipboardEndpointError::Refused(self)
    }
}

impl core::fmt::Display for ClipboardEndpointError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SourceNotEndpoint => formatter.write_str("clipboard-endpoint-source-invalid"),
            Self::RequestRejected => formatter.write_str("clipboard-endpoint-request-rejected"),
            Self::Refused(refusal) => formatter.write_str(refusal.code()),
        }
    }
}

impl std::error::Error for ClipboardEndpointError {}

/// The one ordered gate every clipboard endpoint delivery path runs.
///
/// The relationship is re-derived from the Provider's declared channel
/// vocabulary before anything is compared: an attempt that names another
/// channel's slot, purpose, attachment, or source endpoint is refused as a
/// conflicting declaration rather than narrowed into the one it supplied. The
/// gate stops at the first refusal, so the reported stage is always the
/// earliest reason the attempt is not admitted, and evidence is only ever read.
#[allow(
    clippy::too_many_arguments,
    reason = "each argument is one root-admitted input of the gate (R23)"
)]
pub fn admit_clipboard_endpoint(
    endpoints: &ClipboardHostEndpoints,
    consumer: &ResourceRef,
    binding: &ClipboardEndpointBinding,
    fence: &ClipboardEndpointFence,
    evidence: &ClipboardEndpointEvidence,
    source_uid: &ResourceUid,
    consumer_uid: &ResourceUid,
) -> Result<AdmittedClipboardEndpoint, ClipboardEndpointRefusal> {
    let expected = clipboard_endpoint_bindings(endpoints, consumer)
        .ok()
        .and_then(|derived| {
            derived
                .into_iter()
                .find(|declared| declared.role() == binding.role())
        });
    let Some(expected) = expected else {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
            "clipboard-endpoint-undeclared-channel",
        ));
    };
    if expected.request != *binding.request()
        || expected.source_ref != *binding.source_ref()
        || expected.consumer_ref != *binding.consumer_ref()
    {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
            "clipboard-endpoint-request-mismatch",
        ));
    }
    if evidence.zone != *fence.zone() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::IdentityNotAuthorized,
            "clipboard-endpoint-foreign-zone",
        ));
    }
    if evidence.store != *fence.store() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StoreIncarnationMismatch,
            "clipboard-endpoint-store-incarnation-mismatch",
        ));
    }
    match fence.phase() {
        ClipboardEndpointPhase::Revoked => {
            return Err(ClipboardEndpointRefusal::relationship_revoked());
        }
        ClipboardEndpointPhase::Draining => {
            return Err(ClipboardEndpointRefusal::relationship_draining());
        }
        ClipboardEndpointPhase::Admitted => {}
    }
    if evidence.source_generation != fence.source_generation() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-stale-source-generation",
        ));
    }
    if evidence.consumer_generation != fence.consumer_generation() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Admit,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-stale-consumer-generation",
        ));
    }
    if evidence.desired_revision != fence.desired_revision() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-stale-desired-revision",
        ));
    }
    if evidence.sequence != fence.sequence() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Authorize,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-stale-desired-sequence",
        ));
    }
    if evidence.reconnect.get() < fence.minimum_reconnect().get() {
        return Err(ClipboardEndpointRefusal::at(
            AdmissionStage::Activate,
            RefusalReason::StaleAuthority,
            "clipboard-endpoint-stale-reconnect-generation",
        ));
    }
    let key = binding
        .request()
        .key(fence.zone().clone(), source_uid.clone(), consumer_uid.clone())
        .map_err(|_| {
            ClipboardEndpointRefusal::at(
                AdmissionStage::Admit,
                RefusalReason::IdentityNotAuthorized,
                "clipboard-endpoint-identity-rejected",
            )
        })?;
    Ok(AdmittedClipboardEndpoint {
        binding: binding.clone(),
        key,
        fence: fence.clone(),
    })
}

