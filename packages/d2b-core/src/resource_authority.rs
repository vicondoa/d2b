//! The one pure graph-admission evaluator (U6, KTD4).
//!
//! KTD4 puts the graph authorization decision in exactly one place and gives it
//! two enforcement boundaries: this module answers, the daemon composition and
//! the broker each call it. It is a pure function of path-free authority
//! contracts - it never resolves a host path, names a numerical host
//! principal, reads a store, or mutates anything - so the same prior accepted
//! graph always yields the same decision at either boundary.
//!
//! # What it depends on
//!
//! Only `d2b-contracts-resource` and `d2b-contracts-zone-session`. It imports
//! no provider crate, no toolkit, no `d2b-resource-types`, no manager, and no
//! broker, so the answer cannot be reached through a component that also holds
//! provider or runtime state.
//!
//! # An in-process call is not permission (R8, AE15)
//!
//! The decision is made for [`AuthoritySubject`]: the subject that *initiated*
//! the work. [`TransportIdentity`] records where the request arrived and is
//! deliberately not an input to the decision - a nested call that arrives
//! through a privileged transport keeps its initiating subject, so the
//! transport cannot widen a grant or substitute a more privileged identity for
//! the one that started the work. A request that carries only a transport is
//! refused at the same point a request from an ungranted subject is.
//!
//! # Prior accepted state only
//!
//! Every answer is computed against [`AcceptedGraph`], which holds the graph as
//! it was *already* accepted. A candidate that would introduce its own grant
//! therefore cannot authorize its own introduction, whatever it claims to be:
//! it is not in the prior state the decision reads.

use std::collections::BTreeMap;

use d2b_contracts_resource::v3::credential_binding::CREDENTIAL_BINDING_RESOURCE_TYPE;
use d2b_contracts_resource::v3::device_binding::DEVICE_BINDING_RESOURCE_TYPE;
use d2b_contracts_resource::v3::endpoint_binding::ENDPOINT_BINDING_RESOURCE_TYPE;
use d2b_contracts_resource::v3::network_binding::NETWORK_BINDING_RESOURCE_TYPE;
use d2b_contracts_resource::v3::volume_binding::VOLUME_BINDING_RESOURCE_TYPE;
use d2b_contracts_resource::v3::BindingSourceDecision;
use d2b_contracts_resource::v3::credential_binding::CredentialBindingSpec;
use d2b_contracts_resource::v3::device_binding::DeviceBindingSpec;
use d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec;
use d2b_contracts_resource::v3::network_binding::NetworkBindingSpec;
use d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec;
use d2b_contracts_resource::v3::{
    admit_binding_request, AdmissionDecision, AdmissionStage, AuthoritySubject, BindingAdmission,
    BindingAuthorization, BindingKey, BindingRefusal, BindingRealizationFacet,
    BindingRealizationSupport, CanonicalJsonObject, FreshnessTuple, RefusalReason, RequestedRights,
    ResourceRef, ResourceUid, SourceAdmission, StoreIncarnation, ZoneId,
};
use d2b_contracts_zone_session::v3::role::{AuthorizedRole, ROLE_RESOURCE_TYPE};
use d2b_contracts_zone_session::v3::role_binding::ROLE_BINDING_RESOURCE_TYPE;
use d2b_contracts_zone_session::v3::{RoleBindingSpec, RoleResourceVerb, RoleRule};

/// One decoded row of the projection the broker accepted.
///
/// The broker stores the projection, so it holds each row's canonical admitted
/// bytes rather than a summary the candidate could shape. This is the
/// borrowed view of one such row: the exact reference it was accepted for,
/// and the bytes themselves.
#[derive(Debug, Clone, Copy)]
pub struct ProjectionRow<'a> {
    reference: &'a ResourceRef,
    admitted: &'a CanonicalJsonObject,
    identity: Option<(&'a ResourceUid, &'a ResourceUid)>,
}

impl<'a> ProjectionRow<'a> {
    /// Borrow one row's reference and canonical admitted bytes.
    pub const fn new(reference: &'a ResourceRef, admitted: &'a CanonicalJsonObject) -> Self {
        Self {
            reference,
            admitted,
            identity: None,
        }
    }

    /// A row whose relationship identity the projection resolved: the source
    /// and consumer row uids this row's [`BindingKey`] folds in.
    ///
    /// A binding row names its source and consumer by reference, but a key is
    /// over committed identity, so a rename cannot produce a second
    /// relationship. A row without this is not a binding relationship this
    /// graph can admit, and that is an absence, which refuses.
    pub const fn with_identity(
        reference: &'a ResourceRef,
        admitted: &'a CanonicalJsonObject,
        source_uid: &'a ResourceUid,
        consumer_uid: &'a ResourceUid,
    ) -> Self {
        Self {
            reference,
            admitted,
            identity: Some((source_uid, consumer_uid)),
        }
    }

    /// Borrow the resolved source and consumer identity, when this row has it.
    pub const fn identity(&self) -> Option<(&'a ResourceUid, &'a ResourceUid)> {
        self.identity
    }

    /// The exact resource the row was accepted for.
    pub const fn reference(&self) -> &ResourceRef {
        self.reference
    }

    /// The row's canonical admitted bytes.
    pub const fn admitted(&self) -> &'a CanonicalJsonObject {
        self.admitted
    }
}

/// What class of authority one projected row is.
///
/// The classification reads the canonical type-name constants in the contract
/// crate that declares them, so no shared crate keeps a private copy of the
/// names it decides on. A row outside the three classes contributes nothing to
/// the prior graph: the graph holds the authorization facts, and the broker's
/// own projection stores the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityRowKind {
    /// An accepted `Role`: the rules one grant draws on.
    Role,
    /// An accepted `RoleBinding`: one subject's grant shape.
    RoleBinding,
    /// A committed binding row: the source provider's accepted decision about
    /// one relationship, recoverable from the row alone.
    Binding,
    /// A row that is not authority this evaluator reads.
    Other,
}

impl AuthorityRowKind {
    /// The class one reference carries, read from its own canonical name.
    pub fn of_reference(reference: &ResourceRef) -> Self {
        let name = reference.resource_type().as_str();
        if name == ROLE_RESOURCE_TYPE {
            Self::Role
        } else if name == ROLE_BINDING_RESOURCE_TYPE {
            Self::RoleBinding
        } else if is_binding_resource_type(name) {
            Self::Binding
        } else {
            Self::Other
        }
    }

    /// Whether this class is authority the prior graph reads.
    pub const fn is_authority(self) -> bool {
        matches!(self, Self::Role | Self::RoleBinding)
    }
}

impl ProjectionRow<'_> {
    /// The class this row contributes to the prior graph.
    pub fn kind(&self) -> AuthorityRowKind {
        AuthorityRowKind::of_reference(self.reference)
    }
}

/// Why a stored projection cannot be read as a prior accepted graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedGraphError {
    /// Two rows claimed the same resource reference, so which bytes this
    /// graph holds for that reference would be a coin flip.
    DuplicateRow,
    /// A row of an authority resource type carried bytes that do not decode as
    /// that resource's contract. The row is refused rather than decoded with
    /// its authority quietly dropped.
    UndecodableRow,
}

impl core::fmt::Display for AcceptedGraphError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DuplicateRow => f.write_str("the projection holds two rows for one reference"),
            Self::UndecodableRow => {
                f.write_str("an authority row's committed bytes do not decode as its contract")
            }
        }
    }
}

impl std::error::Error for AcceptedGraphError {}

// ---------------------------------------------------------------------------
// Request shape
// ---------------------------------------------------------------------------

/// The surface a request arrived through.
///
/// This is provenance for a diagnostic, never authority: nothing in the
/// decision below reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportIdentity {
    /// The daemon's own composition, acting on a locally originated request.
    Daemon,
    /// An authenticated component session carrying an API request.
    ComponentSession,
    /// The broker's effect-admission boundary (KTD6-KTD7).
    Broker,
    /// A hosted provider component's own session.
    ProviderSession,
    /// The operator's console.
    OperatorConsole,
}

impl core::fmt::Display for TransportIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TransportIdentity {
    /// The stable label a diagnostic renders.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::ComponentSession => "component-session",
            Self::Broker => "broker",
            Self::ProviderSession => "provider-session",
            Self::OperatorConsole => "operator-console",
        }
    }
}

/// What one durable graph mutation changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MutationKind {
    Create,
    UpdateSpec,
    UpdateMetadata,
    Delete,
}

impl MutationKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 4] =
        [Self::Create, Self::UpdateSpec, Self::UpdateMetadata, Self::Delete];

    /// The stable label a diagnostic renders.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::UpdateSpec => "update-spec",
            Self::UpdateMetadata => "update-metadata",
            Self::Delete => "delete",
        }
    }

    /// The authorization verb this mutation needs.
    ///
    /// Selecting an authority resource is not self-authorization: a `Role` or
    /// `RoleBinding` mutation is still a create or an update-spec on that
    /// resource type, so it is evaluated against the prior accepted graph like
    /// every other mutation.
    pub const fn required_verb(self) -> RoleResourceVerb {
        match self {
            Self::Create => RoleResourceVerb::Create,
            Self::UpdateSpec => RoleResourceVerb::UpdateSpec,
            Self::UpdateMetadata => RoleResourceVerb::UpdateMetadata,
            Self::Delete => RoleResourceVerb::Delete,
        }
    }
}

/// The subject one mutation is decided for, and where it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationSubjectEvidence {
    initiating: AuthoritySubject,
    transport: TransportIdentity,
}

impl MutationSubjectEvidence {
    /// Construct evidence for one initiating subject.
    pub const fn new(initiating: AuthoritySubject, transport: TransportIdentity) -> Self {
        Self { initiating, transport }
    }

    /// The subject the decision is made for.
    pub const fn initiating(&self) -> &AuthoritySubject {
        &self.initiating
    }

    /// Where the request arrived. Recorded for diagnostics only.
    pub const fn transport(&self) -> TransportIdentity {
        self.transport
    }
}

/// One durable graph mutation, as the manager boundary presents it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphMutation {
    zone: ZoneId,
    subject: MutationSubjectEvidence,
    kind: MutationKind,
    target: ResourceRef,
}

impl GraphMutation {
    /// Construct one mutation request.
    pub fn new(
        zone: ZoneId,
        subject: MutationSubjectEvidence,
        kind: MutationKind,
        target: ResourceRef,
    ) -> Self {
        Self { zone, subject, kind, target }
    }

    /// The Zone the mutation belongs to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The subject evidence.
    pub const fn subject(&self) -> &MutationSubjectEvidence {
        &self.subject
    }

    /// What the mutation changes.
    pub const fn kind(&self) -> MutationKind {
        self.kind
    }

    /// The exact resource the mutation names.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }
}

/// One exact binding admission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingAdmissionRequest {
    key: BindingKey,
    rights: RequestedRights,
    required_facets: Vec<BindingRealizationFacet>,
    authorization: BindingAuthorization,
    dependencies: Vec<FreshnessTuple>,
}

impl BindingAdmissionRequest {
    /// Construct one request.
    pub fn new(
        key: BindingKey,
        rights: RequestedRights,
        required_facets: Vec<BindingRealizationFacet>,
        authorization: BindingAuthorization,
        dependencies: Vec<FreshnessTuple>,
    ) -> Self {
        Self { key, rights, required_facets, authorization, dependencies }
    }

    /// The exact relationship this request is for.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// The right it asks for.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// The realization facets it depends on.
    pub fn required_facets(&self) -> &[BindingRealizationFacet] {
        &self.required_facets
    }

    /// The grant evidence the Role evaluation produced.
    pub const fn authorization(&self) -> &BindingAuthorization {
        &self.authorization
    }

    /// The dependency versions the admission is fenced against.
    pub fn dependencies(&self) -> &[FreshnessTuple] {
        &self.dependencies
    }
}

// ---------------------------------------------------------------------------
// Prior accepted graph
// ---------------------------------------------------------------------------

/// One accepted source decision and the realization its implementation
/// declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedSource {
    admission: SourceAdmission,
    support: BindingRealizationSupport,
}

impl AcceptedSource {
    /// Construct one accepted source fact.
    pub const fn new(admission: SourceAdmission, support: BindingRealizationSupport) -> Self {
        Self { admission, support }
    }

    /// The source's own decision.
    pub const fn admission(&self) -> &SourceAdmission {
        &self.admission
    }

    /// What the selected realization declares it can realize.
    pub const fn support(&self) -> &BindingRealizationSupport {
        &self.support
    }
}

/// The prior accepted graph both boundaries evaluate against.
///
/// Everything here was committed and accepted before the request arrived. The
/// Zone, the store incarnation, and the deployment root are what make "prior"
/// checkable: a graph from a different store generation is a different graph,
/// not an older but current one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedGraph {
    zone: ZoneId,
    store: StoreIncarnation,
    root_subject: AuthoritySubject,
    roles: BTreeMap<ResourceRef, AuthorizedRole>,
    role_bindings: BTreeMap<ResourceRef, RoleBindingSpec>,
    sources: BTreeMap<BindingKey, AcceptedSource>,
}

impl AcceptedGraph {
    /// Construct the accepted facts for one store generation.
    pub fn new(
        zone: ZoneId,
        store: StoreIncarnation,
        root_subject: AuthoritySubject,
    ) -> Self {
        Self {
            zone,
            store,
            root_subject,
            roles: BTreeMap::new(),
            role_bindings: BTreeMap::new(),
            sources: BTreeMap::new(),
        }
    }

    /// Record one accepted `Role` row.
    pub fn with_role(mut self, role_ref: ResourceRef, role: AuthorizedRole) -> Self {
        self.roles.insert(role_ref, role);
        self
    }

    /// Record one accepted `RoleBinding` row.
    pub fn with_role_binding(mut self, binding_ref: ResourceRef, binding: RoleBindingSpec) -> Self {
        self.role_bindings.insert(binding_ref, binding);
        self
    }

    /// Record one accepted source decision, keyed by the exact relationship it
    /// was made for.
    pub fn with_source(mut self, source: AcceptedSource) -> Self {
        let key = source.admission.binding().clone();
        self.sources.insert(key, source);
        self
    }

    /// The Zone this graph describes.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The store generation these facts were accepted in.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// The verified deployment graph's own identity.
    pub const fn root_subject(&self) -> &AuthoritySubject {
        &self.root_subject
    }

    /// The accepted authorization rules of one Role.
    pub fn role(&self, role_ref: &ResourceRef) -> Option<&AuthorizedRole> {
        self.roles.get(role_ref)
    }

    /// The accepted grant shape of one RoleBinding.
    pub fn role_binding(&self, binding_ref: &ResourceRef) -> Option<&RoleBindingSpec> {
        self.role_bindings.get(binding_ref)
    }

    /// The accepted source decision for one exact relationship.
    pub fn source(&self, key: &BindingKey) -> Option<&AcceptedSource> {
        self.sources.get(key)
    }

    /// Whether this graph's deployment root is exactly `subject`.
    pub fn is_root(&self, subject: &AuthoritySubject) -> bool {
        &self.root_subject == subject
    }

    /// Read a stored projection as the prior accepted graph (KTD7).
    ///
    /// The broker stores the projection, so this is the one place its stored
    /// rows become an [`AcceptedGraph`] the evaluator reads. It is additive:
    /// it constructs the same value [`Self::with_role`] and
    /// [`Self::with_role_binding`] construct, from bytes rather than from
    /// already-decoded values, so a graph built here decides exactly what a
    /// graph built by hand from the same rows decides.
    ///
    /// A row is classified by the resource type's own canonical constant in
    /// the contract crate that declares it, never by a private table of type
    /// name strings. A row of an authority resource type whose bytes do not
    /// decode as that contract is refused rather than decoded with its
    /// authority dropped, and two rows for one reference are refused rather
    /// than resolved by insertion order. A row of any other type contributes
    /// nothing here: the graph holds the authorization facts, and the broker's
    /// own projection stores the rest.
    pub fn from_canonical_rows<'a>(
        zone: ZoneId,
        store: StoreIncarnation,
        root_subject: AuthoritySubject,
        rows: impl IntoIterator<Item = ProjectionRow<'a>>,
    ) -> Result<Self, AcceptedGraphError> {
        let mut graph = Self::new(zone.clone(), store, root_subject);
        for row in rows {
            // The canonical bytes are re-derived from the decoded object
            // rather than carried beside it, so the contract row is decoded
            // from exactly the bytes the digest covers.
            let bytes = row.admitted.to_canonical_bytes();
            if row.kind() == AuthorityRowKind::Role {
                if graph.roles.contains_key(row.reference) {
                    return Err(AcceptedGraphError::DuplicateRow);
                }
                let role: AuthorizedRole = serde_json::from_slice(&bytes)
                    .map_err(|_| AcceptedGraphError::UndecodableRow)?;
                graph.roles.insert(row.reference.clone(), role);
            } else if row.kind() == AuthorityRowKind::RoleBinding {
                if graph.role_bindings.contains_key(row.reference) {
                    return Err(AcceptedGraphError::DuplicateRow);
                }
                let binding: RoleBindingSpec = serde_json::from_slice(&bytes)
                    .map_err(|_| AcceptedGraphError::UndecodableRow)?;
                graph.role_bindings.insert(row.reference.clone(), binding);
            } else if row.kind() == AuthorityRowKind::Binding {
                // A binding row IS the source provider's accepted decision, so
                // a boundary can rebuild the accepted source fact from it. A
                // row whose identity the projection did not resolve carries no
                // key, contributes nothing, and is an absence - which refuses.
                if let Some((source_uid, consumer_uid)) = row.identity() {
                    let (key, decision) =
                        decode_binding_source(row.reference, &bytes, &zone, source_uid, consumer_uid)?;
                    if graph.sources.contains_key(&key) {
                        return Err(AcceptedGraphError::DuplicateRow);
                    }
                    let admission = SourceAdmission::new(
                        key.clone(),
                        decision.admitted_rights().to_vec(),
                        decision.arbitration(),
                    )
                    .map_err(|_| AcceptedGraphError::UndecodableRow)?;
                    let support = BindingRealizationSupport::new(decision.realized_facets().to_vec())
                        .map_err(|_| AcceptedGraphError::UndecodableRow)?;
                    graph
                        .sources
                        .insert(key, AcceptedSource::new(admission, support));
                }
            }
        }
        Ok(graph)
    }
}

/// Whether one ResourceType names a typed binding row.
fn is_binding_resource_type(name: &str) -> bool {
    matches!(
        name,
        VOLUME_BINDING_RESOURCE_TYPE
            | DEVICE_BINDING_RESOURCE_TYPE
            | ENDPOINT_BINDING_RESOURCE_TYPE
            | NETWORK_BINDING_RESOURCE_TYPE
            | CREDENTIAL_BINDING_RESOURCE_TYPE
    )
}

/// Rebuild one binding row's accepted source fact.
///
/// Each family is decoded through its own contract, never through a shape
/// guessed here, so a row can never be admitted under a spelling the contract
/// does not define.
fn decode_binding_source(
    reference: &ResourceRef,
    bytes: &[u8],
    zone: &ZoneId,
    source_uid: &ResourceUid,
    consumer_uid: &ResourceUid,
) -> Result<(BindingKey, BindingSourceDecision), AcceptedGraphError> {
    /// One decoded row: its relationship key and its source's decision.
    type Facts = Result<(BindingKey, BindingSourceDecision), AcceptedGraphError>;

    let volume = || -> Facts {
        let row: VolumeBindingSpec = decode_binding(bytes)?;
        Ok((map_key(row.key(zone.clone(), source_uid.clone(), consumer_uid.clone()))?, row.source().clone()))
    };
    let device = || -> Facts {
        let row: DeviceBindingSpec = decode_binding(bytes)?;
        Ok((map_key(row.key(zone.clone(), source_uid.clone(), consumer_uid.clone()))?, row.source().clone()))
    };
    let endpoint = || -> Facts {
        let row: EndpointBindingSpec = decode_binding(bytes)?;
        Ok((map_key(row.key(zone.clone(), source_uid.clone(), consumer_uid.clone()))?, row.source().clone()))
    };
    let network = || -> Facts {
        let row: NetworkBindingSpec = decode_binding(bytes)?;
        Ok((map_key(row.key(zone.clone(), source_uid.clone(), consumer_uid.clone()))?, row.source().clone()))
    };
    let credential = || -> Facts {
        let row: CredentialBindingSpec = decode_binding(bytes)?;
        Ok((map_key(row.key(zone.clone(), source_uid.clone(), consumer_uid.clone()))?, row.source().clone()))
    };

    let outcome = match reference.resource_type().as_str() {
        VOLUME_BINDING_RESOURCE_TYPE => volume(),
        DEVICE_BINDING_RESOURCE_TYPE => device(),
        ENDPOINT_BINDING_RESOURCE_TYPE => endpoint(),
        NETWORK_BINDING_RESOURCE_TYPE => network(),
        CREDENTIAL_BINDING_RESOURCE_TYPE => credential(),
        _ => return Err(AcceptedGraphError::UndecodableRow),
    };
    let (key, decision) = outcome?;
    Ok((key, decision))
}

/// Map a binding key derivation failure onto the graph's refusal type.
fn map_key<E>(key: Result<BindingKey, E>) -> Result<BindingKey, AcceptedGraphError> {
    key.map_err(|_| AcceptedGraphError::UndecodableRow)
}

/// Decode one binding row through its own family contract.
fn decode_binding<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, AcceptedGraphError> {
    serde_json::from_slice(bytes).map_err(|_| AcceptedGraphError::UndecodableRow)
}

// ---------------------------------------------------------------------------
// The evaluator
// ---------------------------------------------------------------------------

/// The one pure admission evaluator (KTD4).
///
/// Every method is a pure function of its arguments: no clock, no store, no
/// environment, no I/O.
#[derive(Debug, Clone, Copy, Default)]
pub struct GraphAuthority;

impl GraphAuthority {
    /// Admit one durable graph mutation.
    ///
    /// A mutation is admitted when the initiating subject is authorized for
    /// exactly this verb on exactly this target by the prior accepted graph.
    /// The transport is never consulted (AE15), and a candidate that would add
    /// the grant authorizing it is absent from the prior state by construction.
    pub fn admit_mutation(
        request: &GraphMutation,
        accepted: &AcceptedGraph,
    ) -> AdmissionDecision {
        if request.zone() != accepted.zone() {
            // A Zone the accepted graph does not describe is a different graph,
            // not an older one; only an ownership-bounded reset introduces it.
            return AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::StoreIncarnationMismatch,
            );
        }
        let subject = request.subject().initiating();
        if subject.is_bootstrap_class() {
            // Bootstrap has a fixed trust root, not a configurable exception:
            // only the exact identity the verified deployment graph established
            // is admitted, and only against that graph's own prior state.
            return if accepted.is_root(subject) {
                AdmissionDecision::Admitted
            } else {
                AdmissionDecision::refuse(
                    AdmissionStage::Authorize,
                    RefusalReason::IdentityNotAuthorized,
                )
            };
        }
        let Some(initiating) = subject.resource_ref() else {
            // Only the verified deployment graph and explicit local operator
            // authority are unresourced. Everything else arrives naming the
            // resource it acts as; a subject that names none - which is what a
            // bare privileged transport would leave behind - grants nothing.
            return AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized,
            );
        };
        if accepted
            .role_bindings()
            .any(|(_, binding)| authorizes(binding, accepted, initiating, request))
        {
            AdmissionDecision::Admitted
        } else {
            AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized,
            )
        }
    }

    /// Admit one binding request against the prior accepted graph.
    ///
    /// The grant, the source's own decision, the realization's declared support,
    /// and the dependency fence are all read from the request and the prior
    /// accepted graph. Nothing here widens a request: a right the source does
    /// not admit, a presentation the realization cannot enforce, and an unfenced
    /// admission are each refused.
    pub fn admit_binding(
        request: BindingAdmissionRequest,
        accepted: &AcceptedGraph,
    ) -> Result<BindingAdmission, BindingRefusal> {
        if request.key().zone() != accepted.zone() {
            return Err(BindingRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::StoreIncarnationMismatch,
            ));
        }
        let Some(source) = accepted.source(request.key()) else {
            // No accepted source decision for this exact relationship is not a
            // pending approval: it is an absence, and absence is a refusal.
            return Err(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            ));
        };
        admit_binding_request(
            request.key(),
            request.rights(),
            request.required_facets(),
            request.authorization(),
            source.admission(),
            source.support(),
            request.dependencies(),
        )
    }
}

impl AcceptedGraph {
    /// The accepted RoleBinding rows, in index order.
    pub fn role_bindings(&self) -> impl Iterator<Item = (&ResourceRef, &RoleBindingSpec)> {
        self.role_bindings.iter()
    }

    /// The accepted source facts, in index order.
    pub fn sources(&self) -> impl Iterator<Item = (&BindingKey, &AcceptedSource)> {
        self.sources.iter()
    }
}

/// Whether one accepted RoleBinding authorizes this exact mutation.
fn authorizes(
    binding: &RoleBindingSpec,
    accepted: &AcceptedGraph,
    initiating: &ResourceRef,
    request: &GraphMutation,
) -> bool {
    if !binding.subjects().iter().any(|subject| subject == initiating) {
        return false;
    }
    let Some(role) = accepted.role(binding.role_ref()) else {
        // A binding that names a Role this graph has not accepted carries no
        // rules, so it authorizes nothing.
        return false;
    };
    let verb = request.kind().required_verb();
    let role_rule_admits = role
        .rules()
        .iter()
        .any(|rule| rule_admits(rule, request.target(), request.zone(), verb));
    if !role_rule_admits {
        return false;
    }
    // Every remaining facet is a narrowing: it can only remove authority the
    // Role already granted, never add any.
    if !binding.resource_refs().is_empty()
        && !binding.resource_refs().iter().any(|reference| reference == request.target())
    {
        return false;
    }
    if !binding.zone_refs().is_empty()
        && !binding.zone_refs().iter().any(|zone| zone == request.zone())
    {
        return false;
    }
    if !binding.execution_refs().is_empty()
        && !binding.execution_refs().iter().any(|reference| reference == request.target())
    {
        return false;
    }
    match binding.scope_narrowing() {
        // Narrowing intersects: the request must satisfy the Role through the
        // narrowed rules as well, not merely through the Role's own.
        Some(narrowing) => narrowing
            .rules()
            .iter()
            .any(|rule| rule_admits(rule, request.target(), request.zone(), verb)),
        None => true,
    }
}

/// Whether one Role rule permits this verb on this exact target.
fn rule_admits(rule: &RoleRule, target: &ResourceRef, zone: &ZoneId, verb: RoleResourceVerb) -> bool {
    rule.resource_types()
        .iter()
        .any(|resource_type| resource_type.as_str() == target.resource_type().as_str())
        && rule.verbs().contains(&verb)
        && selects_name(rule.resource_names(), target.name().as_str())
        && selects_zone(rule.zones(), zone)
        && selects_execution(rule.execution_refs(), target)
}

/// Whether an empty selector list is unrestricted or a list is a match.
///
/// An absent selector facet is unrestricted; that is the Role contract's own
/// shape, not an evaluator convenience.
fn selects_name(selectors: &[String], name: &str) -> bool {
    selectors.is_empty() || selectors.iter().any(|selector| selector == "*" || selector == name)
}

fn selects_zone(selectors: &[ZoneId], zone: &ZoneId) -> bool {
    selectors.is_empty() || selectors.contains(zone)
}

fn selects_execution(selectors: &[ResourceRef], target: &ResourceRef) -> bool {
    selectors.is_empty() || selectors.contains(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        AuthoritySubjectKind, BindingArbitration, BindingKind, BindingSlot, ResourceUid,
    };
    use d2b_contracts_zone_session::v3::RoleRule;

    fn zone() -> ZoneId {
        ZoneId::parse("dev").unwrap()
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).unwrap()
    }

    fn uid(last: &str) -> ResourceUid {
        ResourceUid::parse(format!("11111111-1111-4111-8111-{last}")).unwrap()
    }

    fn rule() -> RoleRule {
        RoleRule::new(
            vec![d2b_contracts_resource::v3::ResourceTypeName::parse("VolumeBinding").unwrap()],
            vec![RoleResourceVerb::Create, RoleResourceVerb::Delete],
            Vec::new(),
            vec!["state".to_owned()],
            vec![zone()],
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn role() -> AuthorizedRole {
        AuthorizedRole::new(vec![rule()], Vec::new()).unwrap()
    }

    fn binding() -> RoleBindingSpec {
        RoleBindingSpec::new(
            reference("Role/volume-operator"),
            vec![reference("User/operator")],
            None,
            None,
        )
        .unwrap()
    }

    fn graph() -> AcceptedGraph {
        AcceptedGraph::new(
            zone(),
            StoreIncarnation::parse("store-1").unwrap(),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        )
        .with_role(reference("Role/volume-operator"), role())
        .with_role_binding(reference("RoleBinding/operators"), binding())
    }

    fn mutation(subject_ref: Option<&str>, kind: MutationKind, transport: TransportIdentity) -> GraphMutation {
        let subject = match subject_ref {
            Some(value) => AuthoritySubject::named(
                AuthoritySubjectKind::User,
                reference(value),
            ),
            None => AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        };
        GraphMutation::new(
            zone(),
            MutationSubjectEvidence::new(subject, transport),
            kind,
            reference("VolumeBinding/state"),
        )
    }

    #[test]
    fn a_bound_subject_is_admitted_for_its_exact_verb_and_target() {
        let decision = GraphAuthority::admit_mutation(
            &mutation(Some("User/operator"), MutationKind::Create, TransportIdentity::ComponentSession),
            &graph(),
        );
        assert_eq!(decision, AdmissionDecision::Admitted);
    }

    #[test]
    fn an_unbound_subject_is_refused() {
        let decision = GraphAuthority::admit_mutation(
            &mutation(Some("User/stranger"), MutationKind::Create, TransportIdentity::Daemon),
            &graph(),
        );
        assert_eq!(
            decision,
            AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized
            )
        );
    }

    #[test]
    fn a_privileged_transport_cannot_replace_a_missing_caller_permission() {
        // AE15: the same ungranted subject is refused identically whichever
        // privileged surface it arrives through, and admitted identically once
        // its own grant exists. The transport contributes nothing either way.
        for transport in [
            TransportIdentity::Daemon,
            TransportIdentity::Broker,
            TransportIdentity::ProviderSession,
            TransportIdentity::ComponentSession,
            TransportIdentity::OperatorConsole,
        ] {
            assert_eq!(
                GraphAuthority::admit_mutation(
                    &mutation(Some("User/stranger"), MutationKind::Create, transport),
                    &graph()
                ),
                AdmissionDecision::refuse(
                    AdmissionStage::Authorize,
                    RefusalReason::IdentityNotAuthorized
                ),
                "transport {transport} widened the decision"
            );
            assert_eq!(
                GraphAuthority::admit_mutation(
                    &mutation(Some("User/operator"), MutationKind::Create, transport),
                    &graph()
                ),
                AdmissionDecision::Admitted,
                "transport {transport} changed an admitted decision"
            );
        }
    }

    #[test]
    fn a_publisher_cannot_authorize_its_own_role_binding_introduction() {
        // KTD7: the grant a candidate introduces is absent from the prior
        // accepted state the decision reads, so claiming it - including from
        // the broker boundary, through the most privileged transport - admits
        // nothing.
        let request = GraphMutation::new(
            zone(),
            MutationSubjectEvidence::new(
                AuthoritySubject::unresourced(AuthoritySubjectKind::Operator),
                TransportIdentity::Broker,
            ),
            MutationKind::Create,
            reference("RoleBinding/publisher"),
        );
        assert_eq!(
            GraphAuthority::admit_mutation(&request, &graph()),
            AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized
            )
        );
    }

    #[test]
    fn only_the_exact_deployment_root_is_admitted_without_a_grant() {
        let root = AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap);
        let other_root = AuthoritySubject::unresourced(AuthoritySubjectKind::Operator);
        assert_eq!(
            GraphAuthority::admit_mutation(
                &GraphMutation::new(
                    zone(),
                    MutationSubjectEvidence::new(root.clone(), TransportIdentity::Daemon),
                    MutationKind::Create,
                    reference("Role/bootstrap"),
                ),
                &graph()
            ),
            AdmissionDecision::Admitted
        );
        assert_eq!(
            GraphAuthority::admit_mutation(
                &GraphMutation::new(
                    zone(),
                    MutationSubjectEvidence::new(other_root, TransportIdentity::Daemon),
                    MutationKind::Create,
                    reference("Role/bootstrap"),
                ),
                &graph()
            ),
            AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized
            )
        );
    }

    #[test]
    fn a_mutation_outside_the_bound_name_or_verb_is_refused() {
        for request in [
            GraphMutation::new(
                zone(),
                MutationSubjectEvidence::new(
                    AuthoritySubject::named(
                        AuthoritySubjectKind::User,
                        reference("User/operator"),
                    ),
                    TransportIdentity::ComponentSession,
                ),
                MutationKind::Create,
                reference("VolumeBinding/other"),
            ),
            GraphMutation::new(
                zone(),
                MutationSubjectEvidence::new(
                    AuthoritySubject::named(
                        AuthoritySubjectKind::User,
                        reference("User/operator"),
                    ),
                    TransportIdentity::ComponentSession,
                ),
                MutationKind::UpdateMetadata,
                reference("VolumeBinding/state"),
            ),
        ] {
            assert_eq!(
                GraphAuthority::admit_mutation(&request, &graph()),
                AdmissionDecision::refuse(
                    AdmissionStage::Authorize,
                    RefusalReason::IdentityNotAuthorized
                )
            );
        }
    }

    #[test]
    fn a_graph_from_another_zone_is_a_different_graph() {
        let request = GraphMutation::new(
            ZoneId::parse("prod").unwrap(),
            MutationSubjectEvidence::new(
                AuthoritySubject::named(AuthoritySubjectKind::User, reference("User/operator")),
                TransportIdentity::ComponentSession,
            ),
            MutationKind::Create,
            reference("VolumeBinding/state"),
        );
        assert_eq!(
            GraphAuthority::admit_mutation(&request, &graph()),
            AdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::StoreIncarnationMismatch
            )
        );
    }

    fn binding_key() -> BindingKey {
        BindingKey::new(
            zone(),
            BindingKind::Volume,
            reference("Volume/data"),
            uid("000000000001"),
            reference("Guest/vm"),
            uid("000000000002"),
            BindingSlot::parse("state").unwrap(),
        )
        .unwrap()
    }

    fn freshness() -> FreshnessTuple {
        FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse("store-1").unwrap(),
            reference("Volume/data"),
            uid("000000000001"),
            d2b_contracts_resource::v3::DesiredRevision::INITIAL.try_next().unwrap(),
            d2b_contracts_resource::v3::DesiredDigest::of(b"{}"),
        )
    }

    fn accepted_source(rights: Vec<RequestedRights>) -> AcceptedSource {
        AcceptedSource::new(
            SourceAdmission::new(
                binding_key(),
                rights,
                BindingArbitration::Shared,
            )
            .unwrap(),
            BindingRealizationSupport::new(vec![
                BindingRealizationFacet::FilesystemPresentation,
            ])
            .unwrap(),
        )
    }

    #[test]
    fn a_binding_with_no_accepted_source_decision_is_refused() {
        let request = BindingAdmissionRequest::new(
            binding_key(),
            RequestedRights::Observe,
            vec![BindingRealizationFacet::FilesystemPresentation],
            BindingAuthorization::granted(),
            vec![freshness()],
        );
        assert_eq!(
            GraphAuthority::admit_binding(request, &graph()).unwrap_err().reason(),
            RefusalReason::SourcePolicyRefused
        );
    }

    #[test]
    fn a_binding_whose_source_refuses_the_right_is_refused() {
        let accepted = graph().with_source(accepted_source(vec![RequestedRights::Observe]));
        let request = BindingAdmissionRequest::new(
            binding_key(),
            RequestedRights::Mutate,
            vec![BindingRealizationFacet::FilesystemPresentation],
            BindingAuthorization::granted(),
            vec![freshness()],
        );
        assert_eq!(
            GraphAuthority::admit_binding(request, &accepted).unwrap_err().reason(),
            RefusalReason::SourcePolicyRefused
        );
    }

    #[test]
    fn a_binding_whose_realization_cannot_enforce_the_presentation_is_refused() {
        let accepted = graph().with_source(accepted_source(vec![RequestedRights::Observe]));
        let request = BindingAdmissionRequest::new(
            binding_key(),
            RequestedRights::Observe,
            vec![BindingRealizationFacet::CredentialDelivery],
            BindingAuthorization::granted(),
            vec![freshness()],
        );
        assert_eq!(
            GraphAuthority::admit_binding(request, &accepted).unwrap_err().reason(),
            RefusalReason::MandatoryFacetUnsupported
        );
    }

    #[test]
    fn an_admitted_binding_is_fenced_against_its_dependency_versions() {
        let accepted = graph().with_source(accepted_source(vec![RequestedRights::Observe]));
        let admission = GraphAuthority::admit_binding(
            BindingAdmissionRequest::new(
                binding_key(),
                RequestedRights::Observe,
                vec![BindingRealizationFacet::FilesystemPresentation],
                BindingAuthorization::granted(),
                vec![freshness()],
            ),
            &accepted,
        )
        .unwrap();
        assert!(admission.is_current(&[freshness()]));
        assert!(!admission.is_current(&[]));
    }

    #[test]
    fn a_binding_without_grant_evidence_is_refused() {
        let accepted = graph().with_source(accepted_source(vec![RequestedRights::Observe]));
        let request = BindingAdmissionRequest::new(
            binding_key(),
            RequestedRights::Observe,
            vec![BindingRealizationFacet::FilesystemPresentation],
            BindingAuthorization::absent(),
            vec![freshness()],
        );
        assert_eq!(
            GraphAuthority::admit_binding(request, &accepted).unwrap_err().reason(),
            RefusalReason::IdentityNotAuthorized
        );
    
    }
    ///
    /// This is the regression that made every leg-bearing admission refuse:
    /// `sources` was populated only by a builder with no production caller, so
    /// `admit_binding` saw an absence and refused - correctly, for a fact that
    /// was in fact committed. A boundary must be able to rebuild the accepted
    /// source from the row alone.
    #[test]
    fn a_committed_binding_row_rebuilds_its_accepted_source() {
        let decision = d2b_contracts_resource::v3::BindingSourceDecision::new(
            vec![RequestedRights::Consume],
            BindingArbitration::Shared,
            vec![BindingRealizationFacet::DeviceAttachment],
        )
        .expect("decision validates");
        let spec = DeviceBindingSpec::new(
            reference("Device/gpu0"),
            reference("Process/worker"),
            d2b_contracts_resource::v3::device_binding::DeviceFunction::parse("gpu0")
                .expect("function"),
            d2b_contracts_resource::v3::device_binding::DeviceClaimRequest::Exclusive,
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("slot0").unwrap(),
            decision,
        )
        .expect("a device binding row validates");
        let bytes = CanonicalJsonObject::parse(
            &d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(&spec)
                .expect("the row renders canonically"),
        )
        .expect("the canonical bytes are a JSON object");

        let source_uid = ResourceUid::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let consumer_uid = ResourceUid::parse("22222222-2222-4222-8222-222222222222").unwrap();
        let row_ref = reference("DeviceBinding/gpu0");
        let graph = AcceptedGraph::from_canonical_rows(
            zone(),
            StoreIncarnation::parse("store-1").unwrap(),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            std::iter::once(ProjectionRow::with_identity(
                &row_ref,
                &bytes,
                &source_uid,
                &consumer_uid,
            )),
        )
        .expect("the committed row rebuilds an accepted source");

        let key = spec
            .key(zone(), source_uid, consumer_uid)
            .expect("the row derives its own key");
        let source = graph.source(&key).expect("the rebuilt source is present");
        assert_eq!(source.admission().admitted_rights(), &[RequestedRights::Consume]);
        assert!(source.support().realizes(BindingRealizationFacet::DeviceAttachment));
    }

    /// A row whose identity the projection did not resolve contributes
    /// nothing. Absence is a refusal, not an admission.
    #[test]
    fn a_binding_row_without_resolved_identity_admits_nothing() {
        let decision = d2b_contracts_resource::v3::BindingSourceDecision::new(
            vec![RequestedRights::Consume],
            BindingArbitration::Shared,
            vec![BindingRealizationFacet::DeviceAttachment],
        )
        .expect("decision validates");
        let spec = DeviceBindingSpec::new(
            reference("Device/gpu0"),
            reference("Process/worker"),
            d2b_contracts_resource::v3::device_binding::DeviceFunction::parse("gpu0")
                .expect("function"),
            d2b_contracts_resource::v3::device_binding::DeviceClaimRequest::Exclusive,
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("slot0").unwrap(),
            decision,
        )
        .expect("a device binding row validates");
        let bytes = CanonicalJsonObject::parse(
            &d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(&spec)
                .expect("the row renders canonically"),
        )
        .expect("the canonical bytes are a JSON object");
        let row_ref = reference("DeviceBinding/gpu0");
        let graph = AcceptedGraph::from_canonical_rows(
            zone(),
            StoreIncarnation::parse("store-1").unwrap(),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            std::iter::once(ProjectionRow::new(&row_ref, &bytes)),
        )
        .expect("an unresolved row is not a decode failure");
        assert_eq!(graph.sources().count(), 0, "an unresolved row carries no accepted source");
    }
}
