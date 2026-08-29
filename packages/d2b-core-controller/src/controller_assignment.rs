//! Single-owner Provider controller assignment and fenced ResourceClient leases.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use d2b_contracts_provider::v3::{
    ComponentType, ControllerInstanceScope, ControllerTargetKind, ProviderManifest,
};
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::process::PROCESS_RESOURCE_TYPE;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, PlacementAnchor, PlacementTarget,
    PlacementTargetKind, ResourceEnvelope, ResourceGeneration, ResourceName, ResourceRef,
    ResourceTypeName, ResourceUid, ZoneId, ZoneRevision,
};
use serde_json::{Map, Value, json};

/// Maximum encoded assignment evidence carried by one scoped commit.
pub const MAX_SCOPED_COMMIT_TRANSPORT_BYTES: usize = 64 * 1024;

/// The maximum number of assignments held by one Zone authority.
pub const MAX_ASSIGNMENTS: usize = 16_384;
/// Maximum child ownership entries retained by one assignment.
pub const MAX_ASSIGNED_CHILDREN: usize = 4_096;
/// Assignment-bound query filter for the primary resource UID.
pub const ASSIGNMENT_UID_FILTER: &str = "assignment.resourceUid";
/// Assignment-bound query filter for an owned child resource UID.
pub const OWNER_UID_FILTER: &str = "owner.resourceUid";

/// A monotonically increasing, nonzero assignment epoch.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssignmentEpoch(u64);

impl AssignmentEpoch {
    /// Construct a nonzero epoch.
    pub fn new(value: u64) -> Result<Self, AssignmentError> {
        if value == 0 {
            return Err(AssignmentError::EpochExhausted);
        }
        Ok(Self(value))
    }

    /// Return the opaque epoch ordinal.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for AssignmentEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AssignmentEpoch(<redacted>)")
    }
}

/// The exact target selected by a contract-owned placement anchor.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssignmentTarget {
    /// The Zone singleton target.
    Zone(ZoneId),
    /// One exact Host or Guest execution target.
    Execution {
        /// The target kind.
        kind: PlacementTargetKind,
        /// The exact target reference.
        reference: ResourceRef,
    },
}

impl AssignmentTarget {
    fn from_placement(target: PlacementTarget) -> Self {
        match target {
            PlacementTarget::Zone(zone) => Self::Zone(zone),
            PlacementTarget::Execution { kind, reference } => Self::Execution { kind, reference },
        }
    }

    fn target_kind(&self) -> Option<ControllerTargetKind> {
        match self {
            Self::Zone(_) => Some(ControllerTargetKind::Zone),
            Self::Execution {
                kind: PlacementTargetKind::Host,
                ..
            } => Some(ControllerTargetKind::Host),
            Self::Execution {
                kind: PlacementTargetKind::Guest,
                ..
            } => Some(ControllerTargetKind::Guest),
        }
    }

    /// Borrow the exact execution target reference, when present.
    pub fn execution_ref(&self) -> Option<&ResourceRef> {
        match self {
            Self::Zone(_) => None,
            Self::Execution { reference, .. } => Some(reference),
        }
    }
}

impl fmt::Debug for AssignmentTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zone(_) => formatter.write_str("AssignmentTarget::Zone(<redacted>)"),
            Self::Execution { kind, .. } => formatter
                .debug_struct("AssignmentTarget::Execution")
                .field("kind", kind)
                .finish_non_exhaustive(),
        }
    }
}

/// One resource-plane operation a controller lease may perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssignmentVerb {
    Get,
    List,
    Watch,
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
    CommitBatch,
}

impl AssignmentVerb {
    /// Whether this operation can mutate durable resource state.
    pub const fn is_mutating(self) -> bool {
        !matches!(self, Self::Get | Self::List | Self::Watch)
    }
}

/// Lifecycle of one resource's controller assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentPhase {
    Pending,
    Assigned,
    Draining,
    Revoked,
    Stale,
    Quarantined,
    Released,
}

impl AssignmentPhase {
    const fn code(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Assigned => 1,
            Self::Draining => 2,
            Self::Revoked => 3,
            Self::Stale => 4,
            Self::Quarantined => 5,
            Self::Released => 6,
        }
    }

    fn from_code(value: u8) -> Self {
        match value {
            1 => Self::Assigned,
            2 => Self::Draining,
            3 => Self::Revoked,
            4 => Self::Stale,
            5 => Self::Quarantined,
            6 => Self::Released,
            _ => Self::Pending,
        }
    }

    const fn admits_watch(self) -> bool {
        matches!(self, Self::Assigned)
    }

    const fn admits_mutation(self) -> bool {
        matches!(self, Self::Assigned)
    }
}

/// Closed assignment or lease failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentError {
    InvalidRole,
    ProviderGenerationMismatch,
    ControllerGenerationMismatch,
    SessionGenerationInvalid,
    SessionBindingMismatch,
    ResourceTypeUnowned,
    PlacementAnchorMissing,
    PlacementTargetInvalid,
    TargetKindUnsupported,
    TargetNotReady,
    AssignmentConflict,
    AssignmentLimit,
    AssignmentMissing,
    AssignmentNotDraining,
    AssignmentNotReleased,
    ChildrenRemain,
    ChildLimit,
    StaleAssignment,
    SessionRevoked,
    ResourceRevisionMismatch,
    ResourceUidMismatch,
    ResourceNotAssigned,
    TargetMismatch,
    VerbNotAllowed,
    QueryWidened,
    EpochExhausted,
    RoleContractInvalid,
}

/// Failure while decoding the assignment evidence carried by the existing
/// Resource CommitBatch transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentTransportError {
    Malformed,
    TooLarge,
}

impl AssignmentTransportError {
    /// Return the stable identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Malformed => "assignment-transport-malformed",
            Self::TooLarge => "assignment-transport-too-large",
        }
    }
}

impl fmt::Display for AssignmentTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AssignmentTransportError {}

impl AssignmentError {
    /// Return the stable identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRole => "assignment-role-invalid",
            Self::ProviderGenerationMismatch => "assignment-provider-generation-mismatch",
            Self::ControllerGenerationMismatch => "assignment-controller-generation-mismatch",
            Self::SessionGenerationInvalid => "assignment-session-generation-invalid",
            Self::SessionBindingMismatch => "assignment-session-binding-mismatch",
            Self::ResourceTypeUnowned => "assignment-resource-type-unowned",
            Self::PlacementAnchorMissing => "assignment-placement-anchor-missing",
            Self::PlacementTargetInvalid => "assignment-placement-target-invalid",
            Self::TargetKindUnsupported => "assignment-target-kind-unsupported",
            Self::TargetNotReady => "assignment-target-not-ready",
            Self::AssignmentConflict => "assignment-conflict",
            Self::AssignmentLimit => "assignment-limit",
            Self::AssignmentMissing => "assignment-missing",
            Self::AssignmentNotDraining => "assignment-not-draining",
            Self::AssignmentNotReleased => "assignment-not-released",
            Self::ChildrenRemain => "assignment-children-remain",
            Self::ChildLimit => "assignment-child-limit",
            Self::StaleAssignment => "assignment-stale",
            Self::SessionRevoked => "assignment-session-revoked",
            Self::ResourceRevisionMismatch => "assignment-resource-revision-mismatch",
            Self::ResourceUidMismatch => "assignment-resource-uid-mismatch",
            Self::ResourceNotAssigned => "assignment-resource-not-assigned",
            Self::TargetMismatch => "assignment-target-mismatch",
            Self::VerbNotAllowed => "assignment-verb-not-allowed",
            Self::QueryWidened => "assignment-query-widened",
            Self::EpochExhausted => "assignment-epoch-exhausted",
            Self::RoleContractInvalid => "assignment-role-contract-invalid",
        }
    }
}

impl fmt::Display for AssignmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AssignmentError {}

/// An immutable assignment identity carried by every controller admission.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssignmentIdentity {
    resource_uid: ResourceUid,
    resource_revision: ZoneRevision,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    controller_role: ResourceRef,
    target: AssignmentTarget,
    session_generation: ReconnectGeneration,
    epoch: AssignmentEpoch,
}

impl AssignmentIdentity {
    /// Construct one identity from authoritative committed values.
    #[allow(clippy::too_many_arguments)]
    fn new(
        resource_uid: ResourceUid,
        resource_revision: ZoneRevision,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        controller_role: ResourceRef,
        target: AssignmentTarget,
        session_generation: ReconnectGeneration,
        epoch: AssignmentEpoch,
    ) -> Self {
        Self {
            resource_uid,
            resource_revision,
            provider_generation,
            controller_generation,
            controller_role,
            target,
            session_generation,
            epoch,
        }
    }
}

/// Assignment and mutation evidence forwarded through the existing Resource
/// CommitBatch RPC after bus admission.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedCommitTransport {
    assignment: AssignmentIdentity,
    mutations: Vec<ScopedResourceMutation>,
}

impl ScopedCommitTransport {
    /// Construct transport evidence from one admitted assignment call.
    pub fn new(
        assignment: AssignmentIdentity,
        mutations: Vec<ScopedResourceMutation>,
    ) -> Result<Self, AssignmentTransportError> {
        if mutations.is_empty()
            || mutations.len() > 128
            || mutations.iter().any(|mutation| {
                mutation.assignment() != &assignment || !transport_mutation_is_valid(mutation)
            })
        {
            return Err(AssignmentTransportError::Malformed);
        }
        Ok(Self {
            assignment,
            mutations,
        })
    }

    /// Borrow the admitted assignment.
    pub const fn assignment(&self) -> &AssignmentIdentity {
        &self.assignment
    }

    /// Borrow the admitted mutations.
    pub fn mutations(&self) -> &[ScopedResourceMutation] {
        &self.mutations
    }

    /// Encode the evidence as bounded canonical JSON bytes.
    pub fn encode(&self) -> Result<Vec<u8>, AssignmentTransportError> {
        let value = json!({
            "version": 1,
            "assignment": encode_assignment(&self.assignment),
            "mutations": self
                .mutations
                .iter()
                .map(encode_mutation)
                .collect::<Vec<_>>(),
        });
        let bytes = serde_json::to_vec(&value).map_err(|_| AssignmentTransportError::Malformed)?;
        let bytes = CanonicalJsonValue::parse(&bytes)
            .map_err(|_| AssignmentTransportError::Malformed)?
            .to_canonical_bytes();
        if bytes.len() > MAX_SCOPED_COMMIT_TRANSPORT_BYTES {
            return Err(AssignmentTransportError::TooLarge);
        }
        Ok(bytes)
    }

    /// Decode bounded evidence produced by [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, AssignmentTransportError> {
        if bytes.is_empty() || bytes.len() > MAX_SCOPED_COMMIT_TRANSPORT_BYTES {
            return Err(AssignmentTransportError::TooLarge);
        }
        CanonicalJsonValue::parse(bytes).map_err(|_| AssignmentTransportError::Malformed)?;
        let value = serde_json::from_slice::<Value>(bytes)
            .map_err(|_| AssignmentTransportError::Malformed)?;
        let object = value
            .as_object()
            .ok_or(AssignmentTransportError::Malformed)?;
        require_exact_keys(object, &["version", "assignment", "mutations"])?;
        if object.get("version").and_then(Value::as_u64) != Some(1) {
            return Err(AssignmentTransportError::Malformed);
        }
        let assignment = decode_assignment(
            object
                .get("assignment")
                .ok_or(AssignmentTransportError::Malformed)?,
        )?;
        let mutation_values = object
            .get("mutations")
            .and_then(Value::as_array)
            .ok_or(AssignmentTransportError::Malformed)?;
        if mutation_values.is_empty() || mutation_values.len() > 128 {
            return Err(AssignmentTransportError::Malformed);
        }
        let mutations = mutation_values
            .iter()
            .map(|value| {
                let object = value
                    .as_object()
                    .ok_or(AssignmentTransportError::Malformed)?;
                let scope = match object.get("scope") {
                    None => ScopedResourceScope::Primary,
                    Some(scope) => decode_scoped_resource_scope(scope)?,
                };
                if matches!(scope, ScopedResourceScope::Primary) {
                    require_exact_keys(object, &["target", "verb"])?;
                } else {
                    require_exact_keys(object, &["target", "verb", "scope"])?;
                }
                let target = ResourceRef::parse(
                    object
                        .get("target")
                        .and_then(Value::as_str)
                        .ok_or(AssignmentTransportError::Malformed)?,
                )
                .map_err(|_| AssignmentTransportError::Malformed)?;
                let verb = decode_assignment_verb(
                    object
                        .get("verb")
                        .and_then(Value::as_str)
                        .ok_or(AssignmentTransportError::Malformed)?,
                )?;
                Ok(ScopedResourceMutation {
                    assignment: assignment.clone(),
                    target,
                    verb,
                    scope,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(assignment, mutations)
    }
}

fn transport_mutation_is_valid(mutation: &ScopedResourceMutation) -> bool {
    match mutation.scope() {
        ScopedResourceScope::Primary => matches!(
            mutation.verb(),
            AssignmentVerb::Create
                | AssignmentVerb::UpdateStatus
                | AssignmentVerb::UpdateFinalizers
        ),
        ScopedResourceScope::OwnerChild(scope) => {
            scope.owner_uid() == mutation.assignment().resource_uid()
                && scope.owner_revision() == mutation.assignment().resource_revision()
                && mutation.target().resource_type().as_str() == PROCESS_RESOURCE_TYPE
                && matches!(
                    mutation.verb(),
                    AssignmentVerb::Create
                        | AssignmentVerb::UpdateSpec
                        | AssignmentVerb::Delete
                )
        }
    }
}

impl fmt::Debug for ScopedCommitTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedCommitTransport")
            .field("mutation_count", &self.mutations.len())
            .finish()
    }
}

fn encode_assignment(identity: &AssignmentIdentity) -> Value {
    let target = match identity.target() {
        AssignmentTarget::Zone(zone) => json!({
            "kind": "zone",
            "zone": zone.as_str(),
        }),
        AssignmentTarget::Execution { kind, reference } => json!({
            "kind": "execution",
            "targetKind": match kind {
                PlacementTargetKind::Host => "host",
                PlacementTargetKind::Guest => "guest",
            },
            "reference": reference.to_canonical_string(),
        }),
    };
    json!({
        "resourceUid": identity.resource_uid().as_str(),
        "resourceRevision": identity.resource_revision().get(),
        "providerGeneration": identity.provider_generation().get(),
        "controllerGeneration": identity.controller_generation().get(),
        "controllerRole": identity.controller_role().to_canonical_string(),
        "target": target,
        "sessionGeneration": identity.session_generation().get(),
        "epoch": identity.epoch().get(),
    })
}

fn encode_mutation(mutation: &ScopedResourceMutation) -> Value {
    let mut value = json!({
        "target": mutation.target().to_canonical_string(),
        "verb": encode_assignment_verb(mutation.verb()),
    })
    .as_object()
    .cloned()
    .expect("scoped mutation encoding is an object");
    if let ScopedResourceScope::OwnerChild(scope) = mutation.scope() {
        value.insert("scope".to_owned(), encode_owner_child_scope(scope));
    }
    Value::Object(value)
}

fn encode_owner_child_scope(scope: &OwnerChildScope) -> Value {
    json!({
        "kind": "owner-child",
        "ownerRef": scope.owner_ref().to_canonical_string(),
        "ownerUid": scope.owner_uid().as_str(),
        "ownerRevision": scope.owner_revision().get(),
        "ownerGeneration": scope.owner_generation().get(),
    })
}

fn decode_scoped_resource_scope(
    value: &Value,
) -> Result<ScopedResourceScope, AssignmentTransportError> {
    let object = value
        .as_object()
        .ok_or(AssignmentTransportError::Malformed)?;
    require_exact_keys(
        object,
        &[
            "kind",
            "ownerRef",
            "ownerUid",
            "ownerRevision",
            "ownerGeneration",
        ],
    )?;
    if object.get("kind").and_then(Value::as_str) != Some("owner-child") {
        return Err(AssignmentTransportError::Malformed);
    }
    let owner_ref = ResourceRef::parse(
        object
            .get("ownerRef")
            .and_then(Value::as_str)
            .ok_or(AssignmentTransportError::Malformed)?,
    )
    .map_err(|_| AssignmentTransportError::Malformed)?;
    let owner_uid = ResourceUid::parse(
        object
            .get("ownerUid")
            .and_then(Value::as_str)
            .ok_or(AssignmentTransportError::Malformed)?,
    )
    .map_err(|_| AssignmentTransportError::Malformed)?;
    let owner_revision = ZoneRevision::new(
        object
            .get("ownerRevision")
            .and_then(Value::as_u64)
            .filter(|revision| *revision != 0)
            .ok_or(AssignmentTransportError::Malformed)?,
    );
    let owner_generation = ResourceGeneration::new(
        object
            .get("ownerGeneration")
            .and_then(Value::as_u64)
            .ok_or(AssignmentTransportError::Malformed)?,
    )
    .map_err(|_| AssignmentTransportError::Malformed)?;
    Ok(ScopedResourceScope::OwnerChild(OwnerChildScope {
        owner_ref,
        owner_uid,
        owner_revision,
        owner_generation,
    }))
}

fn encode_assignment_verb(verb: AssignmentVerb) -> &'static str {
    match verb {
        AssignmentVerb::Get => "Get",
        AssignmentVerb::List => "List",
        AssignmentVerb::Watch => "Watch",
        AssignmentVerb::Create => "Create",
        AssignmentVerb::UpdateSpec => "UpdateSpec",
        AssignmentVerb::UpdateStatus => "UpdateStatus",
        AssignmentVerb::UpdateMetadata => "UpdateMetadata",
        AssignmentVerb::UpdateFinalizers => "UpdateFinalizers",
        AssignmentVerb::Delete => "Delete",
        AssignmentVerb::CommitBatch => "CommitBatch",
    }
}

fn decode_assignment_verb(value: &str) -> Result<AssignmentVerb, AssignmentTransportError> {
    match value {
        "Get" => Ok(AssignmentVerb::Get),
        "List" => Ok(AssignmentVerb::List),
        "Watch" => Ok(AssignmentVerb::Watch),
        "Create" => Ok(AssignmentVerb::Create),
        "UpdateSpec" => Ok(AssignmentVerb::UpdateSpec),
        "UpdateStatus" => Ok(AssignmentVerb::UpdateStatus),
        "UpdateMetadata" => Ok(AssignmentVerb::UpdateMetadata),
        "UpdateFinalizers" => Ok(AssignmentVerb::UpdateFinalizers),
        "Delete" => Ok(AssignmentVerb::Delete),
        "CommitBatch" => Ok(AssignmentVerb::CommitBatch),
        _ => Err(AssignmentTransportError::Malformed),
    }
}

fn decode_assignment(value: &Value) -> Result<AssignmentIdentity, AssignmentTransportError> {
    let object = value
        .as_object()
        .ok_or(AssignmentTransportError::Malformed)?;
    require_exact_keys(
        object,
        &[
            "resourceUid",
            "resourceRevision",
            "providerGeneration",
            "controllerGeneration",
            "controllerRole",
            "target",
            "sessionGeneration",
            "epoch",
        ],
    )?;
    let target = decode_assignment_target(
        object
            .get("target")
            .ok_or(AssignmentTransportError::Malformed)?,
    )?;
    Ok(AssignmentIdentity::new(
        ResourceUid::parse(
            object
                .get("resourceUid")
                .and_then(Value::as_str)
                .ok_or(AssignmentTransportError::Malformed)?,
        )
        .map_err(|_| AssignmentTransportError::Malformed)?,
        ZoneRevision::new(
            object
                .get("resourceRevision")
                .and_then(Value::as_u64)
                .ok_or(AssignmentTransportError::Malformed)?,
        ),
        ResourceGeneration::new(
            object
                .get("providerGeneration")
                .and_then(Value::as_u64)
                .ok_or(AssignmentTransportError::Malformed)?,
        )
        .map_err(|_| AssignmentTransportError::Malformed)?,
        ControllerGeneration::new(
            object
                .get("controllerGeneration")
                .and_then(Value::as_u64)
                .ok_or(AssignmentTransportError::Malformed)?,
        )
        .map_err(|_| AssignmentTransportError::Malformed)?,
        ResourceRef::parse(
            object
                .get("controllerRole")
                .and_then(Value::as_str)
                .ok_or(AssignmentTransportError::Malformed)?,
        )
        .map_err(|_| AssignmentTransportError::Malformed)?,
        target,
        ReconnectGeneration::new(
            object
                .get("sessionGeneration")
                .and_then(Value::as_u64)
                .ok_or(AssignmentTransportError::Malformed)?,
        )
        .map_err(|_| AssignmentTransportError::Malformed)?,
        AssignmentEpoch::new(
            object
                .get("epoch")
                .and_then(Value::as_u64)
                .ok_or(AssignmentTransportError::Malformed)?,
        )
        .map_err(|_| AssignmentTransportError::Malformed)?,
    ))
}

fn decode_assignment_target(value: &Value) -> Result<AssignmentTarget, AssignmentTransportError> {
    let object = value
        .as_object()
        .ok_or(AssignmentTransportError::Malformed)?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(AssignmentTransportError::Malformed)?;
    match kind {
        "zone" => {
            require_exact_keys(object, &["kind", "zone"])?;
            Ok(AssignmentTarget::Zone(
                ZoneId::parse(
                    object
                        .get("zone")
                        .and_then(Value::as_str)
                        .ok_or(AssignmentTransportError::Malformed)?,
                )
                .map_err(|_| AssignmentTransportError::Malformed)?,
            ))
        }
        "execution" => {
            require_exact_keys(object, &["kind", "targetKind", "reference"])?;
            let reference = ResourceRef::parse(
                object
                    .get("reference")
                    .and_then(Value::as_str)
                    .ok_or(AssignmentTransportError::Malformed)?,
            )
            .map_err(|_| AssignmentTransportError::Malformed)?;
            let target_kind = match object
                .get("targetKind")
                .and_then(Value::as_str)
                .ok_or(AssignmentTransportError::Malformed)?
            {
                "host" => PlacementTargetKind::Host,
                "guest" => PlacementTargetKind::Guest,
                _ => return Err(AssignmentTransportError::Malformed),
            };
            if (target_kind == PlacementTargetKind::Host
                && reference.resource_type().as_str() != "Host")
                || (target_kind == PlacementTargetKind::Guest
                    && reference.resource_type().as_str() != "Guest")
            {
                return Err(AssignmentTransportError::Malformed);
            }
            Ok(AssignmentTarget::Execution {
                kind: target_kind,
                reference,
            })
        }
        _ => Err(AssignmentTransportError::Malformed),
    }
}

fn require_exact_keys(
    object: &Map<String, Value>,
    expected: &[&str],
) -> Result<(), AssignmentTransportError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(AssignmentTransportError::Malformed);
    }
    Ok(())
}

impl AssignmentIdentity {
    /// Borrow the assigned resource UID.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    /// Return the committed resource revision bound by this identity.
    pub const fn resource_revision(&self) -> ZoneRevision {
        self.resource_revision
    }

    /// Return the Provider generation bound by this identity.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the Core controller generation bound by this identity.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Borrow the signed controller role.
    pub const fn controller_role(&self) -> &ResourceRef {
        &self.controller_role
    }

    /// Borrow the exact assigned target.
    pub const fn target(&self) -> &AssignmentTarget {
        &self.target
    }

    /// Return the authenticated ComponentSession generation.
    pub const fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    /// Return the assignment epoch.
    pub const fn epoch(&self) -> AssignmentEpoch {
        self.epoch
    }
}

impl fmt::Debug for AssignmentIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssignmentIdentity")
            .field("resource_uid", &"<redacted>")
            .field("resource_revision", &self.resource_revision)
            .field("provider_generation", &self.provider_generation)
            .field("controller_generation", &self.controller_generation)
            .field("controller_role", &"<redacted>")
            .field("target", &self.target)
            .field("session_generation", &self.session_generation)
            .field("epoch", &self.epoch)
            .finish()
    }
}

/// The signed role and placement contract used for assignment admission.
#[derive(Clone, PartialEq, Eq)]
pub struct ControllerRoleContract {
    provider_ref: ResourceRef,
    role_ref: ResourceRef,
    scope: ControllerInstanceScope,
    supported_target_kinds: BTreeSet<ControllerTargetKind>,
    resource_types: BTreeSet<ResourceTypeName>,
    placements: BTreeMap<ResourceTypeName, PlacementAnchor>,
}

impl ControllerRoleContract {
    /// Derive one role contract from a trusted signed Provider manifest.
    pub fn from_signed_manifest(
        provider_ref: ResourceRef,
        role_ref: ResourceRef,
        manifest: &ProviderManifest,
    ) -> Result<Self, AssignmentError> {
        if provider_ref.resource_type().as_str() != "Provider"
            || role_ref.resource_type().as_str() != PROCESS_RESOURCE_TYPE
            || manifest.validate_installation_contract().is_err()
        {
            return Err(AssignmentError::InvalidRole);
        }
        let component = manifest
            .components()
            .iter()
            .find(|component| {
                component.component_type() == ComponentType::Controller
                    && component.component_id().as_str() == role_ref.name().as_str()
            })
            .ok_or(AssignmentError::InvalidRole)?;
        let scope = component
            .instance_scope()
            .ok_or(AssignmentError::RoleContractInvalid)?;
        let mut placements = BTreeMap::new();
        for resource_type in component.exported_resource_types() {
            let binding = manifest
                .binding_for(resource_type)
                .ok_or(AssignmentError::PlacementAnchorMissing)?;
            let anchor = *binding
                .placement_anchor()
                .ok_or(AssignmentError::PlacementAnchorMissing)?;
            placements.insert(resource_type.clone(), anchor);
        }
        Ok(Self {
            provider_ref,
            role_ref,
            scope,
            supported_target_kinds: component.supported_target_kinds().clone(),
            resource_types: component.exported_resource_types().clone(),
            placements,
        })
    }

    /// Borrow the Provider resource selected by this signed role.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the controller role reference.
    pub const fn role_ref(&self) -> &ResourceRef {
        &self.role_ref
    }

    /// Return the closed instance scope.
    pub const fn scope(&self) -> ControllerInstanceScope {
        self.scope
    }

    /// Borrow the exclusively owned ResourceTypes.
    pub const fn resource_types(&self) -> &BTreeSet<ResourceTypeName> {
        &self.resource_types
    }

    fn placement_for(&self, resource_type: &ResourceTypeName) -> Option<PlacementAnchor> {
        self.placements.get(resource_type).copied()
    }

    fn supports_target(&self, target: &AssignmentTarget) -> bool {
        target
            .target_kind()
            .is_some_and(|kind| self.supported_target_kinds.contains(&kind))
    }
}

impl fmt::Debug for ControllerRoleContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControllerRoleContract")
            .field("scope", &self.scope)
            .field("resource_type_count", &self.resource_types.len())
            .field("target_kind_count", &self.supported_target_kinds.len())
            .finish()
    }
}

/// Trusted inputs for one assignment admission.
pub struct AssignmentRequest<'a> {
    resource: &'a ResourceEnvelope,
    role: &'a ControllerRoleContract,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    session_generation: ReconnectGeneration,
    target_ready: bool,
}

impl<'a> AssignmentRequest<'a> {
    /// Bind a committed resource to a signed role and authenticated session.
    pub fn new(
        resource: &'a ResourceEnvelope,
        role: &'a ControllerRoleContract,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        session_generation: ReconnectGeneration,
        target_ready: bool,
    ) -> Self {
        Self {
            resource,
            role,
            provider_generation,
            controller_generation,
            session_generation,
            target_ready,
        }
    }
}

/// The exact owner identity bound to an owner-child admission.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerChildScope {
    owner_ref: ResourceRef,
    owner_uid: ResourceUid,
    owner_revision: ZoneRevision,
    owner_generation: ResourceGeneration,
}

impl OwnerChildScope {
    /// Borrow the exact owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the immutable owner UID.
    pub const fn owner_uid(&self) -> &ResourceUid {
        &self.owner_uid
    }

    /// Return the owner revision captured at assignment time.
    pub const fn owner_revision(&self) -> ZoneRevision {
        self.owner_revision
    }

    /// Return the owner generation captured at assignment time.
    pub const fn owner_generation(&self) -> ResourceGeneration {
        self.owner_generation
    }
}

impl fmt::Debug for OwnerChildScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerChildScope")
            .field("owner_ref", &"<redacted>")
            .field("owner_uid", &"<redacted>")
            .field("owner_revision", &self.owner_revision)
            .field("owner_generation", &self.owner_generation)
            .finish()
    }
}

/// The non-widenable scope of an assignment-scoped query or mutation.
#[derive(Clone, PartialEq, Eq)]
pub enum ScopedResourceScope {
    /// The assigned resource itself.
    Primary,
    /// A child resource owned by the exact assigned resource identity.
    OwnerChild(OwnerChildScope),
}

impl ScopedResourceScope {
    /// Borrow the owner-child scope when this is a child admission.
    pub const fn owner_child(&self) -> Option<&OwnerChildScope> {
        match self {
            Self::Primary => None,
            Self::OwnerChild(scope) => Some(scope),
        }
    }
}

impl fmt::Debug for ScopedResourceScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primary => formatter.write_str("ScopedResourceScope::Primary"),
            Self::OwnerChild(scope) => formatter
                .debug_tuple("ScopedResourceScope::OwnerChild")
                .field(scope)
                .finish(),
        }
    }
}

/// A controller's non-widenable query scope.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedResourceQuery {
    assignment: AssignmentIdentity,
    resource_types: Vec<ResourceTypeName>,
    resource_names: Vec<ResourceName>,
    filters: Vec<ScopedResourceFilter>,
    scope: ScopedResourceScope,
}

impl ScopedResourceQuery {
    /// Borrow the immutable assignment evidence.
    pub const fn assignment(&self) -> &AssignmentIdentity {
        &self.assignment
    }

    /// Borrow the exact ResourceType selector.
    pub fn resource_types(&self) -> &[ResourceTypeName] {
        &self.resource_types
    }

    /// Borrow the exact resource-name selector.
    pub fn resource_names(&self) -> &[ResourceName] {
        &self.resource_names
    }

    /// Borrow the assignment-bound filters.
    pub fn filters(&self) -> &[ScopedResourceFilter] {
        &self.filters
    }

    /// Borrow the exact scope minted by the assignment lease.
    pub const fn scope(&self) -> &ScopedResourceScope {
        &self.scope
    }

    /// Borrow the owner-child scope when this is an owner-child query.
    pub const fn owner_child_scope(&self) -> Option<&OwnerChildScope> {
        self.scope.owner_child()
    }

    /// Consume the query while retaining its exact scope.
    pub fn into_parts_with_scope(
        self,
    ) -> (
        AssignmentIdentity,
        Vec<ResourceTypeName>,
        Vec<ResourceName>,
        Vec<ScopedResourceFilter>,
        ScopedResourceScope,
    ) {
        (
            self.assignment,
            self.resource_types,
            self.resource_names,
            self.filters,
            self.scope,
        )
    }
}

impl fmt::Debug for ScopedResourceQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedResourceQuery")
            .field("resource_type_count", &self.resource_types.len())
            .field("resource_name_count", &self.resource_names.len())
            .field("filter_count", &self.filters.len())
            .finish()
    }
}

/// One exact filter minted by a controller assignment.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedResourceFilter {
    field: String,
    values: Vec<String>,
    assignment_bound: bool,
}

impl ScopedResourceFilter {
    /// Construct a caller-supplied narrowing filter. It cannot name the
    /// assignment field; Core appends that filter itself.
    pub fn narrow(field: impl Into<String>, values: Vec<String>) -> Result<Self, AssignmentError> {
        let field = field.into();
        if field.is_empty()
            || field.len() > 64
            || !field
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
            || values.is_empty()
            || values.len() > 64
            || values.iter().any(|value| {
                value.is_empty()
                    || value.len() > 128
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            })
            || matches!(field.as_str(), ASSIGNMENT_UID_FILTER | OWNER_UID_FILTER)
        {
            return Err(AssignmentError::QueryWidened);
        }
        Ok(Self {
            field,
            values,
            assignment_bound: false,
        })
    }

    /// Borrow the indexed field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Borrow the accepted values.
    pub fn values(&self) -> &[String] {
        &self.values
    }

    /// Whether this filter was minted by the assignment authority.
    pub const fn assignment_bound(&self) -> bool {
        self.assignment_bound
    }
}

impl fmt::Debug for ScopedResourceFilter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedResourceFilter")
            .field("field", &self.field)
            .field("value_count", &self.values.len())
            .field("assignment_bound", &self.assignment_bound)
            .finish()
    }
}

/// A single-resource mutation admitted by a controller lease.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopedResourceMutation {
    assignment: AssignmentIdentity,
    target: ResourceRef,
    verb: AssignmentVerb,
    scope: ScopedResourceScope,
}

impl ScopedResourceMutation {
    /// Borrow the assignment evidence.
    pub const fn assignment(&self) -> &AssignmentIdentity {
        &self.assignment
    }

    /// Borrow the exact target.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Return the admitted verb.
    pub const fn verb(&self) -> AssignmentVerb {
        self.verb
    }

    /// Borrow the exact scope minted by the assignment lease.
    pub const fn scope(&self) -> &ScopedResourceScope {
        &self.scope
    }
}

impl fmt::Debug for ScopedResourceMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedResourceMutation")
            .field("target", &"<redacted>")
            .field("verb", &self.verb)
            .field("scope", &self.scope)
            .finish()
    }
}

/// A non-clonable ResourceClient capability minted for one assignment.
pub struct ResourceClientLease {
    identity: AssignmentIdentity,
    resource_ref: ResourceRef,
    resource_generation: ResourceGeneration,
    resource_types: BTreeSet<ResourceTypeName>,
    state: Arc<AssignmentLeaseState>,
    allowed_verbs: BTreeSet<AssignmentVerb>,
}

impl ResourceClientLease {
    /// Borrow the complete immutable assignment identity.
    pub const fn identity(&self) -> &AssignmentIdentity {
        &self.identity
    }

    /// Borrow the exact assigned resource target.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Return the assigned resource generation bound to child admissions.
    pub const fn resource_generation(&self) -> ResourceGeneration {
        self.resource_generation
    }

    /// Borrow the exact assigned placement target.
    pub const fn target(&self) -> &AssignmentTarget {
        self.identity.target()
    }

    /// Return the current lease phase.
    pub fn phase(&self) -> AssignmentPhase {
        AssignmentPhase::from_code(self.state.phase.load(Ordering::Acquire))
    }

    fn ensure_watch(&self) -> Result<(), AssignmentError> {
        let phase = self.phase();
        if phase.admits_watch() {
            return Ok(());
        }
        Err(match phase {
            AssignmentPhase::Revoked => AssignmentError::SessionRevoked,
            AssignmentPhase::Stale
            | AssignmentPhase::Draining
            | AssignmentPhase::Released
            | AssignmentPhase::Quarantined => AssignmentError::StaleAssignment,
            AssignmentPhase::Pending => AssignmentError::AssignmentMissing,
            AssignmentPhase::Assigned => AssignmentError::StaleAssignment,
        })
    }

    fn ensure_mutation(&self) -> Result<(), AssignmentError> {
        let phase = self.phase();
        if phase.admits_mutation() {
            return Ok(());
        }
        Err(match phase {
            AssignmentPhase::Revoked => AssignmentError::SessionRevoked,
            _ => AssignmentError::StaleAssignment,
        })
    }

    /// Mint a query whose assignment filter cannot be removed or widened.
    pub fn query(
        &self,
        resource_types: Vec<ResourceTypeName>,
        resource_names: Vec<ResourceName>,
        filters: Vec<ScopedResourceFilter>,
    ) -> Result<ScopedResourceQuery, AssignmentError> {
        self.ensure_watch()?;
        if resource_types
            .iter()
            .any(|resource_type| !self.resource_types.contains(resource_type))
        {
            return Err(AssignmentError::QueryWidened);
        }
        if filters
            .iter()
            .any(|filter| {
                matches!(
                    filter.field(),
                    ASSIGNMENT_UID_FILTER | OWNER_UID_FILTER
                )
            })
        {
            return Err(AssignmentError::QueryWidened);
        }
        let mut filters = filters;
        filters.push(ScopedResourceFilter {
            field: ASSIGNMENT_UID_FILTER.to_owned(),
            values: vec![self.identity.resource_uid().as_str().to_owned()],
            assignment_bound: true,
        });
        Ok(ScopedResourceQuery {
            assignment: self.identity.clone(),
            resource_types,
            resource_names,
            filters,
            scope: ScopedResourceScope::Primary,
        })
    }

    /// Mint a query limited to Process children owned by this assignment.
    pub fn child_query(
        &self,
        resource_types: Vec<ResourceTypeName>,
        resource_names: Vec<ResourceName>,
        filters: Vec<ScopedResourceFilter>,
    ) -> Result<ScopedResourceQuery, AssignmentError> {
        self.ensure_watch()?;
        if resource_types.is_empty()
            || resource_types
                .iter()
                .any(|resource_type| resource_type.as_str() != PROCESS_RESOURCE_TYPE)
            || filters.iter().any(|filter| {
                matches!(
                    filter.field(),
                    ASSIGNMENT_UID_FILTER | OWNER_UID_FILTER
                )
            })
        {
            return Err(AssignmentError::QueryWidened);
        }
        let owner_scope = self.owner_child_scope();
        let mut filters = filters;
        filters.push(ScopedResourceFilter {
            field: OWNER_UID_FILTER.to_owned(),
            values: vec![owner_scope.owner_uid().as_str().to_owned()],
            assignment_bound: true,
        });
        Ok(ScopedResourceQuery {
            assignment: self.identity.clone(),
            resource_types,
            resource_names,
            filters,
            scope: ScopedResourceScope::OwnerChild(owner_scope),
        })
    }

    /// Admit a mutation against the resource owned by this lease.
    pub fn mutation(
        &self,
        target: ResourceRef,
        verb: AssignmentVerb,
    ) -> Result<ScopedResourceMutation, AssignmentError> {
        self.ensure_mutation()?;
        if verb == AssignmentVerb::CommitBatch || !self.allowed_verbs.contains(&verb) {
            return Err(AssignmentError::VerbNotAllowed);
        }
        if target != self.resource_ref {
            return Err(AssignmentError::ResourceNotAssigned);
        }
        Ok(ScopedResourceMutation {
            assignment: self.identity.clone(),
            target,
            verb,
            scope: ScopedResourceScope::Primary,
        })
    }

    /// Admit a mutation against one Process child owned by this lease.
    ///
    /// Successful commit receipts must be handed to
    /// [`ControllerAssignmentRegistry::record_child`] and
    /// [`ControllerAssignmentRegistry::remove_child`] by the controller
    /// owner. Minting this capability does not pre-account a child that may
    /// never commit.
    pub fn child_mutation(
        &self,
        target: ResourceRef,
        verb: AssignmentVerb,
    ) -> Result<ScopedResourceMutation, AssignmentError> {
        self.ensure_mutation()?;
        if !matches!(
            verb,
            AssignmentVerb::Create | AssignmentVerb::UpdateSpec | AssignmentVerb::Delete
        ) {
            return Err(AssignmentError::VerbNotAllowed);
        }
        if target.resource_type().as_str() != PROCESS_RESOURCE_TYPE {
            return Err(AssignmentError::QueryWidened);
        }
        Ok(ScopedResourceMutation {
            assignment: self.identity.clone(),
            target,
            verb,
            scope: ScopedResourceScope::OwnerChild(self.owner_child_scope()),
        })
    }

    fn owner_child_scope(&self) -> OwnerChildScope {
        OwnerChildScope {
            owner_ref: self.resource_ref.clone(),
            owner_uid: self.identity.resource_uid().clone(),
            owner_revision: self.identity.resource_revision(),
            owner_generation: self.resource_generation,
        }
    }

    /// Verify that a placement target remains exactly the admitted target.
    pub fn target_for(&self, target: PlacementTarget) -> Result<(), AssignmentError> {
        if self.target() == &AssignmentTarget::from_placement(target) {
            Ok(())
        } else {
            Err(AssignmentError::TargetMismatch)
        }
    }
}

impl fmt::Debug for ResourceClientLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceClientLease")
            .field("resource_ref", &"<redacted>")
            .field("phase", &self.phase())
            .field("resource_type_count", &self.resource_types.len())
            .field("allowed_verb_count", &self.allowed_verbs.len())
            .finish()
    }
}

struct AssignmentRecord {
    identity: AssignmentIdentity,
    allowed_verbs: BTreeSet<AssignmentVerb>,
    state: Arc<AssignmentLeaseState>,
    children: BTreeSet<ResourceUid>,
}

struct AssignmentLeaseState {
    phase: AtomicU8,
    stale_observation: AtomicBool,
}

impl AssignmentLeaseState {
    fn new(phase: AssignmentPhase) -> Self {
        Self {
            phase: AtomicU8::new(phase.code()),
            stale_observation: AtomicBool::new(false),
        }
    }

    fn phase(&self) -> AssignmentPhase {
        AssignmentPhase::from_code(self.phase.load(Ordering::Acquire))
    }

    fn set_phase(&self, phase: AssignmentPhase) {
        self.phase.store(phase.code(), Ordering::Release);
    }

    fn mark_stale(&self) {
        self.stale_observation.store(true, Ordering::Release);
    }
}

/// Core's single-owner assignment registry.
#[derive(Default)]
pub struct ControllerAssignmentRegistry {
    records: BTreeMap<ResourceUid, AssignmentRecord>,
    active_targets: BTreeMap<(ResourceTypeName, AssignmentTarget), BTreeSet<ResourceUid>>,
    next_epoch: u64,
}

impl fmt::Debug for ControllerAssignmentRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControllerAssignmentRegistry")
            .field("assignment_count", &self.records.len())
            .field("active_target_count", &self.active_targets.len())
            .finish()
    }
}

impl ControllerAssignmentRegistry {
    /// Admit one resource from the committed store snapshot.
    pub fn admit(
        &mut self,
        request: AssignmentRequest<'_>,
    ) -> Result<ResourceClientLease, AssignmentError> {
        let resource_type = request.resource.resource_type().clone();
        if !request.role.resource_types.contains(&resource_type) {
            return Err(AssignmentError::ResourceTypeUnowned);
        }
        if request.resource.spec().provider_ref() != Some(request.role.provider_ref()) {
            return Err(AssignmentError::InvalidRole);
        }
        let placement_anchor = request
            .role
            .placement_for(&resource_type)
            .ok_or(AssignmentError::PlacementAnchorMissing)?;
        let target = AssignmentTarget::from_placement(
            placement_anchor
                .resolve(request.resource)
                .map_err(|_| AssignmentError::PlacementTargetInvalid)?,
        );
        if !request.role.supports_target(&target) {
            return Err(AssignmentError::TargetKindUnsupported);
        }
        if matches!(
            request.role.scope(),
            ControllerInstanceScope::ZoneSingleton
                if !matches!(target, AssignmentTarget::Zone(_))
        ) || matches!(
            request.role.scope(),
            ControllerInstanceScope::FixedExecutionTarget
                if !matches!(target, AssignmentTarget::Execution { .. })
        ) {
            return Err(AssignmentError::RoleContractInvalid);
        }
        if !request.target_ready {
            return Err(AssignmentError::TargetNotReady);
        }
        if let Some(existing) = self.records.get(request.resource.metadata().uid()) {
            if matches!(
                existing.state.phase(),
                AssignmentPhase::Released | AssignmentPhase::Quarantined
            ) {
                self.records.remove(request.resource.metadata().uid());
            } else {
                return Err(AssignmentError::AssignmentConflict);
            }
        }
        if self.records.len() >= MAX_ASSIGNMENTS {
            return Err(AssignmentError::AssignmentLimit);
        }
        let target_key = (resource_type.clone(), target.clone());
        if self
            .active_targets
            .get(&target_key)
            .is_some_and(|uids| !uids.is_empty())
            // A per-resource target role intentionally allows multiple
            // resources at one target. The key is therefore only used for
            // fixed/Zone singleton roles.
            && matches!(
                request.role.scope(),
                ControllerInstanceScope::ZoneSingleton
                    | ControllerInstanceScope::FixedExecutionTarget
            )
        {
            return Err(AssignmentError::AssignmentConflict);
        }
        let epoch_value = self
            .next_epoch
            .checked_add(1)
            .ok_or(AssignmentError::EpochExhausted)?;
        self.next_epoch = epoch_value;
        let epoch = AssignmentEpoch::new(epoch_value)?;
        let identity = AssignmentIdentity::new(
            request.resource.metadata().uid().clone(),
            request.resource.metadata().revision(),
            request.provider_generation,
            request.controller_generation,
            request.role.role_ref().clone(),
            target.clone(),
            request.session_generation,
            epoch,
        );
        let allowed_verbs = BTreeSet::from([
            AssignmentVerb::Get,
            AssignmentVerb::List,
            AssignmentVerb::Watch,
            AssignmentVerb::Create,
            AssignmentVerb::UpdateStatus,
            AssignmentVerb::UpdateFinalizers,
            AssignmentVerb::CommitBatch,
        ]);
        let state = Arc::new(AssignmentLeaseState::new(AssignmentPhase::Assigned));
        self.records.insert(
            identity.resource_uid().clone(),
            AssignmentRecord {
                identity: identity.clone(),
                allowed_verbs: allowed_verbs.clone(),
                state: Arc::clone(&state),
                children: BTreeSet::new(),
            },
        );
        self.active_targets
            .entry(target_key)
            .or_default()
            .insert(identity.resource_uid().clone());
        Ok(ResourceClientLease {
            identity,
            resource_ref: ResourceRef::new(
                resource_type.clone(),
                request.resource.metadata().name().clone(),
            ),
            resource_generation: request.resource.metadata().generation(),
            resource_types: request.role.resource_types.clone(),
            state,
            allowed_verbs,
        })
    }

    /// Rebind one live lease to the resource revision produced by its last
    /// successful write without changing its assignment epoch.
    pub fn rebind_revision(
        &mut self,
        lease: &mut ResourceClientLease,
        revision: ZoneRevision,
    ) -> Result<(), AssignmentError> {
        let record = self
            .records
            .get_mut(lease.identity.resource_uid())
            .ok_or(AssignmentError::AssignmentMissing)?;
        if record.identity != lease.identity {
            return Err(AssignmentError::StaleAssignment);
        }
        if record.state.phase() == AssignmentPhase::Revoked {
            return Err(AssignmentError::SessionRevoked);
        }
        if record.state.phase() != AssignmentPhase::Assigned {
            return Err(AssignmentError::StaleAssignment);
        }
        if revision < lease.identity.resource_revision() {
            return Err(AssignmentError::ResourceRevisionMismatch);
        }
        if revision == lease.identity.resource_revision() {
            return Ok(());
        }
        let mut identity = lease.identity.clone();
        identity.resource_revision = revision;
        record.identity = identity.clone();
        lease.identity = identity;
        Ok(())
    }

    /// Return the current phase for an assignment identity.
    pub fn phase(&self, identity: &AssignmentIdentity) -> Option<AssignmentPhase> {
        self.records
            .get(identity.resource_uid())
            .filter(|record| record.identity == *identity)
            .map(|record| record.state.phase())
    }

    /// Mark an assignment as draining before target or generation handoff.
    pub fn begin_drain(&mut self, identity: &AssignmentIdentity) -> Result<(), AssignmentError> {
        let record = self.record_mut(identity)?;
        if record.state.phase() != AssignmentPhase::Assigned {
            return Err(AssignmentError::AssignmentNotDraining);
        }
        record.state.set_phase(AssignmentPhase::Draining);
        record.state.mark_stale();
        Ok(())
    }

    /// Release a drained or revoked assignment and its target index.
    pub fn release(&mut self, identity: &AssignmentIdentity) -> Result<(), AssignmentError> {
        let record = self.record_mut(identity)?;
        if !matches!(
            record.state.phase(),
            AssignmentPhase::Draining | AssignmentPhase::Revoked | AssignmentPhase::Quarantined
        ) {
            return Err(AssignmentError::AssignmentNotReleased);
        }
        if !record.children.is_empty() {
            return Err(AssignmentError::ChildrenRemain);
        }
        record.state.set_phase(AssignmentPhase::Released);
        self.remove_active_target(identity);
        Ok(())
    }

    /// Quarantine an assignment whose target or child ownership is ambiguous.
    pub fn quarantine(&mut self, identity: &AssignmentIdentity) -> Result<(), AssignmentError> {
        let record = self.record_mut(identity)?;
        record.state.set_phase(AssignmentPhase::Quarantined);
        record.state.mark_stale();
        self.remove_active_target(identity);
        Ok(())
    }

    /// Record one child resource in the assignment's narrow owner index.
    pub fn record_child(
        &mut self,
        identity: &AssignmentIdentity,
        child_uid: ResourceUid,
    ) -> Result<(), AssignmentError> {
        let record = self.record_mut(identity)?;
        if record.children.len() >= MAX_ASSIGNED_CHILDREN {
            return Err(AssignmentError::ChildLimit);
        }
        record.children.insert(child_uid);
        Ok(())
    }

    /// Remove one child after its terminal deletion is committed.
    pub fn remove_child(
        &mut self,
        identity: &AssignmentIdentity,
        child_uid: &ResourceUid,
    ) -> Result<(), AssignmentError> {
        let record = self.record_mut(identity)?;
        if !record.children.remove(child_uid) {
            return Err(AssignmentError::AssignmentMissing);
        }
        Ok(())
    }

    /// Return the currently indexed child UIDs.
    pub fn child_uids(&self, identity: &AssignmentIdentity) -> Option<&BTreeSet<ResourceUid>> {
        self.records
            .get(identity.resource_uid())
            .filter(|record| record.identity == *identity)
            .map(|record| &record.children)
    }

    /// Revoke all assignments bound to a disconnected session generation.
    pub fn revoke_session(&mut self, generation: ReconnectGeneration) {
        for record in self.records.values_mut() {
            if record.identity.session_generation() == generation
                && matches!(
                    record.state.phase(),
                    AssignmentPhase::Assigned | AssignmentPhase::Draining
                )
            {
                record.state.set_phase(AssignmentPhase::Revoked);
                record.state.mark_stale();
            }
        }
    }

    /// Validate a writer against every assignment fence.
    pub fn validate_writer(
        &self,
        identity: &AssignmentIdentity,
        uid: &ResourceUid,
        revision: ZoneRevision,
        verb: AssignmentVerb,
    ) -> Result<(), AssignmentError> {
        let record = self
            .records
            .get(identity.resource_uid())
            .ok_or(AssignmentError::AssignmentMissing)?;
        if record.identity != *identity {
            return Err(AssignmentError::StaleAssignment);
        }
        if record.state.phase() == AssignmentPhase::Revoked {
            return Err(AssignmentError::SessionRevoked);
        }
        if !record.state.phase().admits_mutation() {
            return Err(AssignmentError::StaleAssignment);
        }
        if record.identity.resource_uid() != uid {
            return Err(AssignmentError::ResourceUidMismatch);
        }
        if record.identity.resource_revision() != revision {
            return Err(AssignmentError::ResourceRevisionMismatch);
        }
        if !record.allowed_verbs.contains(&verb) {
            return Err(AssignmentError::VerbNotAllowed);
        }
        Ok(())
    }

    /// Validate a read or mutation lease without a new resource snapshot.
    pub fn validate_scope(
        &self,
        identity: &AssignmentIdentity,
        verb: AssignmentVerb,
    ) -> Result<(), AssignmentError> {
        let record = self
            .records
            .get(identity.resource_uid())
            .ok_or(AssignmentError::AssignmentMissing)?;
        if record.identity != *identity {
            return Err(AssignmentError::StaleAssignment);
        }
        if record.state.phase() == AssignmentPhase::Revoked {
            return Err(AssignmentError::SessionRevoked);
        }
        if !record.state.phase().admits_watch()
            && matches!(
                verb,
                AssignmentVerb::Get | AssignmentVerb::List | AssignmentVerb::Watch
            )
        {
            return Err(AssignmentError::StaleAssignment);
        }
        if !record.state.phase().admits_mutation() && verb.is_mutating() {
            return Err(AssignmentError::StaleAssignment);
        }
        if !record.allowed_verbs.contains(&verb) {
            return Err(AssignmentError::VerbNotAllowed);
        }
        Ok(())
    }

    /// Whether the last committed observation must be retained as stale.
    pub fn observation_is_stale(&self, identity: &AssignmentIdentity) -> bool {
        self.records
            .get(identity.resource_uid())
            .filter(|record| record.identity == *identity)
            .is_some_and(|record| record.state.stale_observation.load(Ordering::Acquire))
    }

    fn record_mut(
        &mut self,
        identity: &AssignmentIdentity,
    ) -> Result<&mut AssignmentRecord, AssignmentError> {
        let record = self
            .records
            .get_mut(identity.resource_uid())
            .ok_or(AssignmentError::AssignmentMissing)?;
        if record.identity != *identity {
            return Err(AssignmentError::StaleAssignment);
        }
        Ok(record)
    }

    fn remove_active_target(&mut self, identity: &AssignmentIdentity) {
        self.active_targets.retain(|_, uids| {
            uids.remove(identity.resource_uid());
            !uids.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_provider::v3::{
        ArtifactDigest, ArtifactDigestSet, BinaryRef, CompatibilityRange, ComponentDescriptor,
        ComponentExecution, ComponentTargetCapability, ComponentType, ControllerInstanceScope,
        ControllerTargetKind, EffectPortClass, PolicyEvaluation, ProviderManifest,
        ResourceApiBinding, RevocationState, SignatureState, TargetRuntimeArtifacts, TrustEvidence,
        UpgradeDisposition, UpgradePolicy,
    };
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::identity::ReconnectGeneration;
    use d2b_contracts_resource::v3::{
        ControllerGeneration, PlacementTarget, ResourceEnvelope, ResourceGeneration, ResourceRef,
        ResourceTypeName, ResourceUid, SchemaFingerprint, SchemaVersion, ZoneRevision,
    };

    use super::{
        AssignmentError, AssignmentPhase, AssignmentRequest, AssignmentTarget, AssignmentVerb,
        ControllerAssignmentRegistry, ControllerRoleContract, PROCESS_RESOURCE_TYPE,
        ScopedCommitTransport, ScopedResourceFilter,
    };

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn digest() -> ArtifactDigest {
        ArtifactDigest::parse(DIGEST).unwrap()
    }

    fn fingerprint() -> SchemaFingerprint {
        SchemaFingerprint::parse(DIGEST).unwrap()
    }

    fn manifest() -> ProviderManifest {
        let process = ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap();
        let component = ComponentDescriptor::new(
            BoundedToken::parse("process-controller").unwrap(),
            ComponentType::Controller,
            [process.clone()],
            [],
            [d2b_contracts_resource::v3::ExecutionDomain::System],
            8,
            digest(),
            [],
            false,
        )
        .unwrap()
        .with_execution(ComponentExecution::Launchable {
            binary_ref: BinaryRef::parse("process-controller").unwrap(),
        })
        .with_controller_placement(
            ControllerInstanceScope::PerResourceTarget,
            [ControllerTargetKind::Host, ControllerTargetKind::Guest],
        )
        .unwrap()
        .with_target_capabilities([
            ComponentTargetCapability::new(
                ControllerTargetKind::Host,
                digest(),
                [EffectPortClass::Process],
            )
            .unwrap(),
            ComponentTargetCapability::new(
                ControllerTargetKind::Guest,
                digest(),
                [EffectPortClass::Process],
            )
            .unwrap(),
        ])
        .unwrap();
        let binding = ResourceApiBinding::new_with_placement(
            process,
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint(),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint(),
            Default::default(),
            None,
            None,
            d2b_contracts_resource::v3::PlacementAnchor::ExecutionRef,
        )
        .unwrap();
        let trust = TrustEvidence {
            publisher: BoundedToken::parse("trusted").unwrap(),
            root_epoch: 1,
            publisher_trusted: true,
            signature: SignatureState::Valid,
            revocation: RevocationState::Clear,
            emergency_deny: false,
            provenance: PolicyEvaluation::Accepted,
            sbom: PolicyEvaluation::Accepted,
            license: PolicyEvaluation::Accepted,
            vulnerability: PolicyEvaluation::Accepted,
            conformance: PolicyEvaluation::Accepted,
            support_channel: BoundedToken::parse("stable").unwrap(),
        };
        ProviderManifest::new(
            d2b_contracts_resource::v3::ArtifactId::parse("provider-runtime").unwrap(),
            ArtifactDigestSet {
                executable: digest(),
                config: digest(),
                schema: digest(),
                service: digest(),
            },
            trust,
            CompatibilityRange {
                api_major: 3,
                api_minor: 0,
                descriptor_fingerprint: fingerprint(),
                state_schema_version: SchemaVersion::new(1, 0).unwrap(),
            },
            [component],
            [binding],
            [],
            UpgradePolicy {
                drain_before_upgrade: true,
                max_automatic_disposition: UpgradeDisposition::InPlace,
                preserves_durable_state: true,
            },
        )
        .unwrap()
        .with_target_runtime_artifacts([
            TargetRuntimeArtifacts::new(ControllerTargetKind::Host, digest(), digest()).unwrap(),
            TargetRuntimeArtifacts::new(ControllerTargetKind::Guest, digest(), digest()).unwrap(),
        ])
        .unwrap()
    }

    fn process(name: &str, execution_ref: &str, revision: u64) -> ResourceEnvelope {
        let uid = if name.contains("guest") {
            if name.contains("second") {
                "323e4567-e89b-42d3-a456-426614174002"
            } else {
                "223e4567-e89b-42d3-a456-426614174001"
            }
        } else {
            "123e4567-e89b-42d3-a456-426614174000"
        };
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": PROCESS_RESOURCE_TYPE,
            "metadata": {
                "name": name,
                "zone": "dev",
                "uid": uid,
                "generation": 1,
                "revision": revision,
                "ownerRef": null,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "2026-07-22T00:00:00.000Z",
                "updatedAt": "2026-07-22T00:00:00.000Z",
                "managedBy": "api",
                "configurationGeneration": null,
                "controllerGeneration": null,
                "providerGeneration": null
            },
            "spec": {
                "providerRef": "Provider/provider-runtime",
                "executionRef": execution_ref,
                "argv": ["true"]
            },
            "status": {
                "completedAt": null,
                "conditions": [],
                "lastReconciledAt": null,
                "observedGeneration": 0,
                "outcome": null,
                "phase": "Pending",
                "resource": {},
                "startedAt": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                }
            }
        });
        ResourceEnvelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn role() -> ControllerRoleContract {
        ControllerRoleContract::from_signed_manifest(
            ResourceRef::parse("Provider/provider-runtime").unwrap(),
            ResourceRef::parse("Process/process-controller").unwrap(),
            &manifest(),
        )
        .unwrap()
    }

    fn request<'a>(
        resource: &'a ResourceEnvelope,
        role: &'a ControllerRoleContract,
        provider_generation: u64,
        controller_generation: u64,
        session_generation: u64,
    ) -> AssignmentRequest<'a> {
        AssignmentRequest::new(
            resource,
            role,
            ResourceGeneration::new(provider_generation).unwrap(),
            ControllerGeneration::new(controller_generation).unwrap(),
            ReconnectGeneration::new(session_generation).unwrap(),
            true,
        )
    }

    #[test]
    fn host_and_guest_resources_have_one_disjoint_target_assignment() {
        let host = process("host-process", "Host/host-system", 11);
        let guest = process("guest-process", "Guest/dev-vm", 12);
        let guest_second = process("guest-process-second", "Guest/dev-vm", 13);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let host_lease = registry.admit(request(&host, &role, 2, 3, 4)).unwrap();
        let guest_lease = registry.admit(request(&guest, &role, 2, 3, 5)).unwrap();
        let guest_second_lease = registry
            .admit(request(&guest_second, &role, 2, 3, 6))
            .unwrap();

        assert_ne!(
            host_lease.identity().target(),
            guest_lease.identity().target()
        );
        assert_eq!(
            guest_lease.identity().target(),
            guest_second_lease.identity().target()
        );
        assert_ne!(
            host_lease.identity().epoch(),
            guest_lease.identity().epoch()
        );
        assert_eq!(host_lease.phase(), AssignmentPhase::Assigned);
        assert_eq!(guest_lease.phase(), AssignmentPhase::Assigned);
        assert_eq!(
            host_lease.target(),
            &AssignmentTarget::Execution {
                kind: d2b_contracts_resource::v3::PlacementTargetKind::Host,
                reference: ResourceRef::parse("Host/host-system").unwrap(),
            }
        );
    }

    #[test]
    fn stale_assignment_epoch_rejects_status_and_finalizer_writers() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let old = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        registry.begin_drain(old.identity()).unwrap();
        registry.release(old.identity()).unwrap();
        let new = registry.admit(request(&resource, &role, 1, 1, 2)).unwrap();

        assert_eq!(
            registry.validate_writer(
                old.identity(),
                &resource.metadata().uid().clone(),
                resource.metadata().revision(),
                AssignmentVerb::UpdateStatus,
            ),
            Err(AssignmentError::StaleAssignment)
        );
        assert!(
            registry
                .validate_writer(
                    new.identity(),
                    &resource.metadata().uid().clone(),
                    resource.metadata().revision(),
                    AssignmentVerb::UpdateFinalizers,
                )
                .is_ok()
        );
    }

    #[test]
    fn scoped_commit_transport_round_trips_assignment_and_mutations() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        let target = ResourceRef::new(
            resource.resource_type().clone(),
            resource.metadata().name().clone(),
        );
        let mutation = lease
            .mutation(target, AssignmentVerb::UpdateStatus)
            .unwrap();
        let transport =
            ScopedCommitTransport::new(lease.identity().clone(), vec![mutation]).unwrap();
        let decoded = ScopedCommitTransport::decode(&transport.encode().unwrap()).unwrap();

        assert_eq!(decoded.assignment(), lease.identity());
        assert_eq!(decoded.mutations(), transport.mutations());
    }

    #[test]
    fn scoped_commit_transport_round_trips_owner_child_scope() {
        let owner = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry.admit(request(&owner, &role, 1, 1, 1)).unwrap();
        let mutation = lease
            .child_mutation(
                ResourceRef::parse("Process/process-vmm").unwrap(),
                AssignmentVerb::Create,
            )
            .unwrap();
        let transport =
            ScopedCommitTransport::new(lease.identity().clone(), vec![mutation]).unwrap();
        let encoded = transport.encode().unwrap();
        let decoded = ScopedCommitTransport::decode(&encoded).unwrap();
        let scope = decoded.mutations()[0].scope().owner_child().unwrap();

        assert_eq!(decoded.assignment(), lease.identity());
        assert_eq!(scope.owner_ref(), lease.resource_ref());
        assert_eq!(scope.owner_uid(), owner.metadata().uid());
        assert_eq!(scope.owner_revision(), owner.metadata().revision());
        assert_eq!(scope.owner_generation(), owner.metadata().generation());
    }

    #[test]
    fn same_epoch_rebind_updates_the_active_writer_revision() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let mut lease = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        let stale = lease.identity().clone();

        registry
            .rebind_revision(&mut lease, ZoneRevision::new(8))
            .unwrap();

        assert_eq!(lease.identity().resource_revision(), ZoneRevision::new(8));
        assert!(
            registry
                .validate_writer(
                    lease.identity(),
                    resource.metadata().uid(),
                    ZoneRevision::new(8),
                    AssignmentVerb::UpdateStatus,
                )
                .is_ok()
        );
        assert_eq!(
            registry.validate_writer(
                &stale,
                resource.metadata().uid(),
                ZoneRevision::new(7),
                AssignmentVerb::UpdateStatus,
            ),
            Err(AssignmentError::StaleAssignment)
        );
    }

    #[test]
    fn released_assignment_allows_successor_at_the_current_revision() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let mut old = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        registry
            .rebind_revision(&mut old, ZoneRevision::new(8))
            .unwrap();
        registry.begin_drain(old.identity()).unwrap();
        registry.release(old.identity()).unwrap();

        let current = process("process", "Guest/dev-vm", 8);
        let successor = registry.admit(request(&current, &role, 2, 2, 2)).unwrap();
        assert_eq!(
            successor.identity().resource_revision(),
            ZoneRevision::new(8)
        );
        assert!(
            registry
                .validate_writer(
                    successor.identity(),
                    current.metadata().uid(),
                    ZoneRevision::new(8),
                    AssignmentVerb::UpdateFinalizers,
                )
                .is_ok()
        );
    }

    #[test]
    fn disconnected_session_revokes_mutation_but_keeps_stale_observation() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry.admit(request(&resource, &role, 1, 1, 9)).unwrap();
        registry.revoke_session(ReconnectGeneration::new(9).unwrap());

        assert_eq!(
            registry.phase(lease.identity()),
            Some(AssignmentPhase::Revoked)
        );
        assert_eq!(lease.phase(), AssignmentPhase::Revoked);
        assert_eq!(
            lease.mutation(
                ResourceRef::parse("Process/process").unwrap(),
                AssignmentVerb::UpdateStatus,
            ),
            Err(AssignmentError::SessionRevoked)
        );
        assert_eq!(
            lease.child_query(
                vec![ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()],
                Vec::new(),
                Vec::new(),
            ),
            Err(AssignmentError::SessionRevoked)
        );
        assert_eq!(
            lease.child_mutation(
                ResourceRef::parse("Process/process-child").unwrap(),
                AssignmentVerb::Create,
            ),
            Err(AssignmentError::SessionRevoked)
        );
        assert_eq!(
            registry.validate_writer(
                lease.identity(),
                resource.metadata().uid(),
                resource.metadata().revision(),
                AssignmentVerb::UpdateStatus,
            ),
            Err(AssignmentError::SessionRevoked)
        );
        assert!(registry.observation_is_stale(lease.identity()));
    }

    #[test]
    fn guest_lease_cannot_widen_to_host_or_foreign_resource() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        let query = lease
            .query(
                vec![ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()],
                vec![],
                vec![
                    ScopedResourceFilter::narrow("metadata.name", vec!["process".to_owned()])
                        .unwrap(),
                ],
            )
            .unwrap();
        assert!(
            query
                .filters()
                .iter()
                .any(|filter| filter.field() == "metadata.name")
        );
        assert!(
            query
                .filters()
                .iter()
                .any(|filter| filter.field() == "assignment.resourceUid"
                    && filter.assignment_bound())
        );
        assert_eq!(
            lease.query(
                vec![ResourceTypeName::parse("Host").unwrap()],
                vec![],
                vec![],
            ),
            Err(AssignmentError::QueryWidened)
        );
        assert_eq!(
            lease.mutation(
                ResourceRef::parse("Process/other").unwrap(),
                AssignmentVerb::UpdateStatus,
            ),
            Err(AssignmentError::ResourceNotAssigned)
        );
        assert_eq!(
            lease.target_for(PlacementTarget::Execution {
                kind: d2b_contracts_resource::v3::PlacementTargetKind::Host,
                reference: ResourceRef::parse("Host/host-system").unwrap(),
            }),
            Err(AssignmentError::TargetMismatch)
        );
    }

    #[test]
    fn assigned_lease_mints_an_exact_process_child_scope() {
        let owner = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry.admit(request(&owner, &role, 1, 1, 1)).unwrap();
        let process_type = ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap();

        let query = lease
            .child_query(vec![process_type.clone()], vec![], vec![])
            .unwrap();
        assert_eq!(query.resource_types(), &[process_type]);
        let owner_filter = query
            .filters()
            .iter()
            .find(|filter| filter.field() == "owner.resourceUid")
            .expect("owner UID filter");
        assert!(owner_filter.assignment_bound());
        assert_eq!(
            owner_filter.values(),
            &[owner.metadata().uid().as_str().to_owned()]
        );
        assert_eq!(
            query.owner_child_scope().unwrap().owner_ref(),
            &ResourceRef::new(
                owner.resource_type().clone(),
                owner.metadata().name().clone()
            )
        );

        let child = lease
            .child_mutation(
                ResourceRef::parse("Process/process-vmm").unwrap(),
                AssignmentVerb::Create,
            )
            .unwrap();
        assert_eq!(child.target(), &ResourceRef::parse("Process/process-vmm").unwrap());
        let child_scope = child.scope().owner_child().unwrap();
        assert_eq!(child_scope.owner_uid(), owner.metadata().uid());
        assert_eq!(
            child_scope.owner_revision(),
            owner.metadata().revision()
        );
        assert_eq!(
            child_scope.owner_generation(),
            owner.metadata().generation()
        );
        assert_eq!(
            lease.child_mutation(
                ResourceRef::parse("Process/process-vmm").unwrap(),
                AssignmentVerb::UpdateStatus,
            ),
            Err(AssignmentError::VerbNotAllowed)
        );
        assert_eq!(
            lease.child_mutation(
                ResourceRef::parse("Host/process-vmm").unwrap(),
                AssignmentVerb::Create,
            ),
            Err(AssignmentError::QueryWidened)
        );
        assert_eq!(
            lease.child_query(Vec::new(), Vec::new(), Vec::new()),
            Err(AssignmentError::QueryWidened)
        );
        assert_eq!(
            lease.child_query(
                vec![ResourceTypeName::parse("Host").unwrap()],
                Vec::new(),
                Vec::new(),
            ),
            Err(AssignmentError::QueryWidened)
        );
        assert_eq!(
            lease.child_mutation(
                ResourceRef::parse("Process/process-vmm").unwrap(),
                AssignmentVerb::UpdateFinalizers,
            ),
            Err(AssignmentError::VerbNotAllowed)
        );
        for field in [super::ASSIGNMENT_UID_FILTER, super::OWNER_UID_FILTER] {
            let forged = ScopedResourceFilter {
                field: field.to_owned(),
                values: vec![owner.metadata().uid().as_str().to_owned()],
                assignment_bound: false,
            };
            assert_eq!(
                lease.child_query(
                    vec![ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()],
                    Vec::new(),
                    vec![forged],
                ),
                Err(AssignmentError::QueryWidened)
            );
        }
        assert_eq!(
            lease.mutation(
                ResourceRef::new(
                    owner.resource_type().clone(),
                    owner.metadata().name().clone()
                ),
                AssignmentVerb::UpdateSpec,
            ),
            Err(AssignmentError::VerbNotAllowed)
        );
        assert_eq!(
            lease.mutation(
                ResourceRef::new(
                    owner.resource_type().clone(),
                    owner.metadata().name().clone()
                ),
                AssignmentVerb::Delete,
            ),
            Err(AssignmentError::VerbNotAllowed)
        );
        registry.begin_drain(lease.identity()).unwrap();
        assert_eq!(
        lease.child_query(
            vec![ResourceTypeName::parse(PROCESS_RESOURCE_TYPE).unwrap()],
            Vec::new(),
            Vec::new(),
        ),
        Err(AssignmentError::StaleAssignment)
        );
        assert_eq!(
        lease.child_mutation(
            ResourceRef::parse("Process/process-vmm").unwrap(),
            AssignmentVerb::Create,
        ),
        Err(AssignmentError::StaleAssignment)
        );
    }

    #[test]
    fn target_handoff_requires_drain_and_release_before_reassignment() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let old = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        assert_eq!(
            registry
                .admit(request(&resource, &role, 2, 2, 2))
                .unwrap_err(),
            AssignmentError::AssignmentConflict
        );
        registry.begin_drain(old.identity()).unwrap();
        assert_eq!(
            registry
                .admit(request(&resource, &role, 2, 2, 2))
                .unwrap_err(),
            AssignmentError::AssignmentConflict
        );
        registry.release(old.identity()).unwrap();
        let replacement = registry.admit(request(&resource, &role, 2, 2, 2)).unwrap();
        assert_eq!(replacement.identity().provider_generation().get(), 2);
        assert_eq!(replacement.identity().controller_generation().get(), 2);
    }

    #[test]
    fn child_index_must_drain_before_parent_release() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry.admit(request(&resource, &role, 1, 1, 1)).unwrap();
        let child = ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap();
        registry
            .record_child(lease.identity(), child.clone())
            .unwrap();
        registry.begin_drain(lease.identity()).unwrap();
        assert_eq!(
            registry.release(lease.identity()),
            Err(AssignmentError::ChildrenRemain)
        );
        assert_eq!(registry.child_uids(lease.identity()).unwrap().len(), 1);
        registry.remove_child(lease.identity(), &child).unwrap();
        registry.release(lease.identity()).unwrap();
    }

    #[test]
    fn ambiguous_or_unready_targets_fail_closed_without_fallback() {
        let resource = process("process", "Guest/dev-vm", 7);
        let role = role();
        let mut registry = ControllerAssignmentRegistry::default();
        assert_eq!(
            registry
                .admit(AssignmentRequest::new(
                    &resource,
                    &role,
                    ResourceGeneration::new(1).unwrap(),
                    ControllerGeneration::new(1).unwrap(),
                    ReconnectGeneration::new(1).unwrap(),
                    false,
                ))
                .unwrap_err(),
            AssignmentError::TargetNotReady
        );
        let invalid = process("process", "User/not-a-target", 7);
        assert_eq!(
            registry
                .admit(request(&invalid, &role, 1, 1, 1))
                .unwrap_err(),
            AssignmentError::PlacementTargetInvalid
        );
    }
}
