//! Cloud Hypervisor child addresses, UID-free creates, and private identity.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use d2b_contracts_resource::v3::{
    ArtifactId, CanonicalJsonValue, DesiredLifecycle, ResourceName, ResourceRef, ResourceTypeName,
    ResourceUid, ZoneId, ZoneRevision, execution_policy::BoundedToken,
    resource_schema::canonical_json_bytes,
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::descriptor::{GuestSetupDescriptorError, VerifiedGuestSetupDescriptor};

/// Domain tag for private Cloud Hypervisor runtime scopes.
pub const PRIVATE_RUNTIME_SCOPE_DOMAIN_TAG: &str = "d2b:v3:ch-private-runtime-scope";
/// Provider used for VMM Process resources.
pub const PROCESS_PROVIDER_REF: &str = "Provider/system-minijail";
/// Provider used for controller-owned setup Volumes.
pub const VOLUME_PROVIDER_REF: &str = "Provider/volume-local";

/// Closed failures while constructing child identities or batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildIdentityError {
    /// The Guest or execution reference named the wrong ResourceType.
    WrongResourceType,
    /// A deterministic child name exceeded the ResourceName bound.
    ChildNameInvalid,
    /// A role was duplicated or the fixed role set was incomplete.
    ChildRolesInvalid,
    /// The descriptor was not valid at the batch boundary.
    DescriptorInvalid,
    /// A canonical child batch could not be encoded.
    CanonicalJson,
    /// A semantic child token was invalid.
    InvalidToken,
    /// A private runtime role was invalid.
    InvalidRuntimeRole,
    /// A committed child revision was zero.
    InvalidRevision,
}

impl fmt::Display for ChildIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongResourceType => "cloud-hypervisor-child-resource-type-invalid",
            Self::ChildNameInvalid => "cloud-hypervisor-child-name-invalid",
            Self::ChildRolesInvalid => "cloud-hypervisor-child-roles-invalid",
            Self::DescriptorInvalid => "cloud-hypervisor-descriptor-invalid",
            Self::CanonicalJson => "cloud-hypervisor-child-batch-canonical-json",
            Self::InvalidToken => "cloud-hypervisor-child-token-invalid",
            Self::InvalidRuntimeRole => "cloud-hypervisor-runtime-role-invalid",
            Self::InvalidRevision => "cloud-hypervisor-child-revision-invalid",
        })
    }
}

impl std::error::Error for ChildIdentityError {}

/// The fixed direct child roles owned by a Cloud Hypervisor Guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChildRole {
    /// The stopped-then-started VMM Process.
    #[serde(rename = "vmm")]
    VmmProcess,
    /// The Cloud Hypervisor API Endpoint.
    #[serde(rename = "ch-api")]
    ChApiEndpoint,
    /// The authenticated guest-control Endpoint.
    #[serde(rename = "guest-control")]
    GuestControlEndpoint,
    /// The controller-owned system/setup Volume.
    #[serde(rename = "system")]
    SystemVolume,
}

impl ChildRole {
    /// Return the ResourceType created for this role.
    pub const fn resource_type(self) -> &'static str {
        match self {
            Self::VmmProcess => "Process",
            Self::ChApiEndpoint | Self::GuestControlEndpoint => "Endpoint",
            Self::SystemVolume => "Volume",
        }
    }

    /// Return the stable name suffix for this role.
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::VmmProcess => "vmm",
            Self::ChApiEndpoint => "ch-api",
            Self::GuestControlEndpoint => "guest-control",
            Self::SystemVolume => "system",
        }
    }

    /// Return the semantic purpose used by an Endpoint child.
    pub const fn purpose(self) -> Option<&'static str> {
        match self {
            Self::ChApiEndpoint => Some("ch-api"),
            Self::GuestControlEndpoint => Some("guest-control"),
            Self::VmmProcess | Self::SystemVolume => None,
        }
    }

    /// Return the semantic Volume view used by a setup Volume child.
    pub const fn volume_view(self) -> Option<&'static str> {
        match self {
            Self::SystemVolume => Some("system"),
            Self::VmmProcess | Self::ChApiEndpoint | Self::GuestControlEndpoint => None,
        }
    }
}

/// The exact closed set of direct Cloud Hypervisor child roles.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ChildRoleSet {
    roles: Vec<ChildRole>,
}

impl ChildRoleSet {
    /// Construct the fixed direct child role set.
    pub fn fixed() -> Self {
        Self {
            roles: vec![
                ChildRole::VmmProcess,
                ChildRole::ChApiEndpoint,
                ChildRole::GuestControlEndpoint,
                ChildRole::SystemVolume,
            ],
        }
    }

    /// Construct and validate a closed child role set.
    pub fn new(roles: impl IntoIterator<Item = ChildRole>) -> Result<Self, ChildIdentityError> {
        let mut roles = roles.into_iter().collect::<Vec<_>>();
        let original_len = roles.len();
        roles.sort();
        roles.dedup();
        let mut expected = Self::fixed().roles;
        expected.sort();
        if roles.len() != original_len || roles != expected {
            return Err(ChildIdentityError::ChildRolesInvalid);
        }
        Ok(Self { roles: expected })
    }

    /// Borrow the fixed roles in stable creation order.
    pub fn iter(&self) -> impl Iterator<Item = ChildRole> + '_ {
        Self::fixed()
            .roles
            .into_iter()
            .filter(|role| self.roles.contains(role))
    }

    /// Whether this set is exactly the fixed direct child set.
    pub fn is_fixed(&self) -> bool {
        self.roles == Self::fixed().roles
    }

    /// Return whether the set contains one role.
    pub fn contains(&self, role: ChildRole) -> bool {
        self.roles.contains(&role)
    }
}

impl fmt::Debug for ChildRoleSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildRoleSet")
            .field("role_count", &self.roles.len())
            .finish()
    }
}

impl<'de> Deserialize<'de> for ChildRoleSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(Vec::<ChildRole>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Compute the deterministic child name for a Guest and fixed role.
pub fn deterministic_child_name(
    guest_ref: &ResourceRef,
    role: ChildRole,
) -> Result<ResourceName, ChildIdentityError> {
    if guest_ref.resource_type().as_str() != "Guest" {
        return Err(ChildIdentityError::WrongResourceType);
    }
    ResourceName::parse(format!("{}-{}", guest_ref.name().as_str(), role.suffix()))
        .map_err(|_| ChildIdentityError::ChildNameInvalid)
}

/// Compute the deterministic Zone-local child ResourceRef.
pub fn deterministic_child_ref(
    guest_ref: &ResourceRef,
    role: ChildRole,
) -> Result<ResourceRef, ChildIdentityError> {
    let name = deterministic_child_name(guest_ref, role)?;
    let resource_type = ResourceTypeName::parse(role.resource_type())
        .map_err(|_| ChildIdentityError::WrongResourceType)?;
    Ok(ResourceRef::new(resource_type, name))
}

/// The U10 CommitBatch precondition required for every first-generation child
/// create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreatePrecondition {
    /// The target must not already exist.
    CreateAbsent,
}

/// UID-free Process child create body.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCreateBody {
    provider_ref: ResourceRef,
    execution_ref: ResourceRef,
    template: BoundedToken,
    desired_lifecycle: DesiredLifecycle,
}

impl ProcessCreateBody {
    /// Construct a stopped VMM Process create body.
    pub fn new(execution_ref: ResourceRef) -> Result<Self, ChildIdentityError> {
        if execution_ref.resource_type().as_str() != "Host" {
            return Err(ChildIdentityError::WrongResourceType);
        }
        Ok(Self {
            provider_ref: ResourceRef::parse(PROCESS_PROVIDER_REF)
                .map_err(|_| ChildIdentityError::WrongResourceType)?,
            execution_ref,
            template: BoundedToken::parse("cloud-hypervisor-runner")
                .map_err(|_| ChildIdentityError::InvalidToken)?,
            desired_lifecycle: DesiredLifecycle::Stopped,
        })
    }

    /// Borrow the Process Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the signed Process template selector.
    pub const fn template(&self) -> &BoundedToken {
        &self.template
    }

    /// Return the initial desired lifecycle.
    pub const fn desired_lifecycle(&self) -> DesiredLifecycle {
        self.desired_lifecycle
    }
}

impl fmt::Debug for ProcessCreateBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessCreateBody(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for ProcessCreateBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            execution_ref: ResourceRef,
            template: BoundedToken,
            desired_lifecycle: DesiredLifecycle,
        }

        let wire = Wire::deserialize(deserializer)?;
        let body = Self::new(wire.execution_ref).map_err(serde::de::Error::custom)?;
        if body.provider_ref != wire.provider_ref
            || body.template != wire.template
            || body.desired_lifecycle != wire.desired_lifecycle
        {
            return Err(serde::de::Error::custom(ChildIdentityError::InvalidToken));
        }
        Ok(body)
    }
}

/// UID-free Endpoint child create body.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointCreateBody {
    provider_ref: ResourceRef,
    producer_ref: ResourceRef,
    purpose: BoundedToken,
}

impl EndpointCreateBody {
    /// Construct a semantic Endpoint create body.
    pub fn new(
        provider_ref: ResourceRef,
        producer_ref: ResourceRef,
        purpose: impl Into<String>,
    ) -> Result<Self, ChildIdentityError> {
        if provider_ref.resource_type().as_str() != "Provider"
            || !matches!(
                producer_ref.resource_type().as_str(),
                "Guest" | "Process" | "EphemeralProcess" | "Device" | "Host"
            )
        {
            return Err(ChildIdentityError::WrongResourceType);
        }
        let purpose =
            BoundedToken::parse(purpose.into()).map_err(|_| ChildIdentityError::InvalidToken)?;
        Ok(Self {
            provider_ref,
            producer_ref,
            purpose,
        })
    }

    /// Borrow the Endpoint Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the Endpoint producer reference.
    pub const fn producer_ref(&self) -> &ResourceRef {
        &self.producer_ref
    }

    /// Borrow the semantic Endpoint purpose.
    pub const fn purpose(&self) -> &BoundedToken {
        &self.purpose
    }
}

impl fmt::Debug for EndpointCreateBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EndpointCreateBody(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for EndpointCreateBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            producer_ref: ResourceRef,
            purpose: BoundedToken,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.provider_ref, wire.producer_ref, wire.purpose.as_str())
            .map_err(serde::de::Error::custom)
    }
}

/// UID-free setup Volume create body bound to the Guest system artifact.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeCreateBody {
    provider_ref: ResourceRef,
    execution_ref: ResourceRef,
    system_artifact_id: ArtifactId,
    view: BoundedToken,
}

impl VolumeCreateBody {
    /// Construct a semantic setup Volume create body.
    pub fn new(
        execution_ref: ResourceRef,
        system_artifact_id: ArtifactId,
        view: impl Into<String>,
    ) -> Result<Self, ChildIdentityError> {
        if execution_ref.resource_type().as_str() != "Host" {
            return Err(ChildIdentityError::WrongResourceType);
        }
        Ok(Self {
            provider_ref: ResourceRef::parse(VOLUME_PROVIDER_REF)
                .map_err(|_| ChildIdentityError::WrongResourceType)?,
            execution_ref,
            system_artifact_id,
            view: BoundedToken::parse(view.into()).map_err(|_| ChildIdentityError::InvalidToken)?,
        })
    }

    /// Borrow the Volume Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the selected Guest system artifact.
    pub const fn system_artifact_id(&self) -> &ArtifactId {
        &self.system_artifact_id
    }

    /// Borrow the semantic Volume view.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }
}

impl fmt::Debug for VolumeCreateBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VolumeCreateBody(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for VolumeCreateBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            provider_ref: ResourceRef,
            execution_ref: ResourceRef,
            system_artifact_id: ArtifactId,
            view: BoundedToken,
        }

        let wire = Wire::deserialize(deserializer)?;
        let body = Self::new(wire.execution_ref, wire.system_artifact_id, wire.view.as_str())
            .map_err(serde::de::Error::custom)?;
        if body.provider_ref != wire.provider_ref {
            return Err(serde::de::Error::custom(
                ChildIdentityError::WrongResourceType,
            ));
        }
        Ok(body)
    }
}

/// The closed UID-free body family used by the child batch.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "spec", rename_all = "camelCase")]
pub enum ChildCreateBody {
    /// Process create body.
    Process(ProcessCreateBody),
    /// Endpoint create body.
    Endpoint(EndpointCreateBody),
    /// Volume create body.
    Volume(VolumeCreateBody),
}

impl fmt::Debug for ChildCreateBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ChildCreateBody")
            .field(match self {
                Self::Process(_) => &"process",
                Self::Endpoint(_) => &"endpoint",
                Self::Volume(_) => &"volume",
            })
            .finish()
    }
}

/// One UID-free CreateAbsent child mutation.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildMutation {
    target: ResourceRef,
    owner_ref: ResourceRef,
    zone: ZoneId,
    precondition: CreatePrecondition,
    body: ChildCreateBody,
}

impl<'de> Deserialize<'de> for ChildMutation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            target: ResourceRef,
            owner_ref: ResourceRef,
            zone: ZoneId,
            precondition: CreatePrecondition,
            body: ChildCreateBody,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.precondition != CreatePrecondition::CreateAbsent {
            return Err(serde::de::Error::custom(
                ChildIdentityError::InvalidToken,
            ));
        }
        Self::new(wire.target, wire.owner_ref, wire.zone, wire.body)
            .map_err(serde::de::Error::custom)
    }
}

impl ChildMutation {
    /// Construct a UID-free child create mutation.
    pub fn new(
        target: ResourceRef,
        owner_ref: ResourceRef,
        zone: ZoneId,
        body: ChildCreateBody,
    ) -> Result<Self, ChildIdentityError> {
        if owner_ref.resource_type().as_str() != "Guest"
            || target.resource_type().as_str()
                != match body {
                    ChildCreateBody::Process(_) => "Process",
                    ChildCreateBody::Endpoint(_) => "Endpoint",
                    ChildCreateBody::Volume(_) => "Volume",
                }
        {
            return Err(ChildIdentityError::WrongResourceType);
        }
        Ok(Self {
            target,
            owner_ref,
            zone,
            precondition: CreatePrecondition::CreateAbsent,
            body,
        })
    }

    /// Borrow the name-addressed child target.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Borrow the Guest owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the child Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the create precondition.
    pub const fn precondition(&self) -> CreatePrecondition {
        self.precondition
    }

    /// Return the absent UID fence on first create.
    pub const fn expected_uid(&self) -> Option<&ResourceUid> {
        None
    }

    /// Borrow the typed create body.
    pub const fn body(&self) -> &ChildCreateBody {
        &self.body
    }
}

impl fmt::Debug for ChildMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildMutation")
            .field("target", &self.target)
            .field("owner_ref", &self.owner_ref)
            .field("precondition", &self.precondition)
            .finish_non_exhaustive()
    }
}

/// One complete related Cloud Hypervisor child batch.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestChildBatch {
    zone: ZoneId,
    owner_ref: ResourceRef,
    child_refs: BTreeMap<ChildRole, ResourceRef>,
    mutations: Vec<ChildMutation>,
}

impl GuestChildBatch {
    /// Construct the complete UID-free child batch from a valid descriptor.
    pub fn from_descriptor(
        zone: ZoneId,
        owner_ref: ResourceRef,
        execution_ref: ResourceRef,
        descriptor: &VerifiedGuestSetupDescriptor,
    ) -> Result<Self, ChildIdentityError> {
        let descriptor = descriptor.descriptor();
        if owner_ref.resource_type().as_str() != "Guest"
            || execution_ref.resource_type().as_str() != "Host"
        {
            return Err(ChildIdentityError::WrongResourceType);
        }
        let process_ref = deterministic_child_ref(&owner_ref, ChildRole::VmmProcess)?;
        let mut child_refs = BTreeMap::new();
        let mut mutations = Vec::new();
        for role in descriptor.child_roles().iter() {
            let target = deterministic_child_ref(&owner_ref, role)?;
            let body = match role {
                ChildRole::VmmProcess => {
                    ChildCreateBody::Process(ProcessCreateBody::new(execution_ref.clone())?)
                }
                ChildRole::ChApiEndpoint => ChildCreateBody::Endpoint(EndpointCreateBody::new(
                    descriptor.provider_ref().clone(),
                    process_ref.clone(),
                    role.purpose().expect("fixed Endpoint purpose"),
                )?),
                ChildRole::GuestControlEndpoint => {
                    ChildCreateBody::Endpoint(EndpointCreateBody::new(
                        descriptor.provider_ref().clone(),
                        owner_ref.clone(),
                        role.purpose().expect("fixed Endpoint purpose"),
                    )?)
                }
                ChildRole::SystemVolume => ChildCreateBody::Volume(VolumeCreateBody::new(
                    execution_ref.clone(),
                    descriptor.system_artifact_id().clone(),
                    role.volume_view().expect("fixed Volume view"),
                )?),
            };
            child_refs.insert(role, target.clone());
            mutations.push(ChildMutation::new(
                target,
                owner_ref.clone(),
                zone.clone(),
                body,
            )?);
        }
        Ok(Self {
            zone,
            owner_ref,
            child_refs,
            mutations,
        })
    }

    /// Borrow the Zone-local owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the child batch Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the deterministic ResourceRef for one fixed role.
    pub fn child_ref(&self, role: ChildRole) -> Option<&ResourceRef> {
        self.child_refs.get(&role)
    }

    /// Borrow the UID-free CreateAbsent mutations in stable role order.
    pub fn mutations(&self) -> &[ChildMutation] {
        &self.mutations
    }

    /// Render the canonical child batch without private descriptor payload.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ChildIdentityError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct BatchWire<'a> {
            zone: &'a ZoneId,
            owner_ref: &'a ResourceRef,
            mutations: &'a [ChildMutation],
        }

        let wire = BatchWire {
            zone: &self.zone,
            owner_ref: &self.owner_ref,
            mutations: &self.mutations,
        };
        let bytes = canonical_json_bytes(&wire).map_err(|_| ChildIdentityError::CanonicalJson)?;
        Ok(bytes)
    }
}

impl fmt::Debug for GuestChildBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestChildBatch")
            .field("owner_ref", &self.owner_ref)
            .field("child_count", &self.mutations.len())
            .finish()
    }
}

/// A resource row returned by a successful child commit.
#[derive(Clone, PartialEq, Eq)]
pub struct CommittedChild {
    resource_ref: ResourceRef,
    owner_ref: ResourceRef,
    zone: ZoneId,
    uid: ResourceUid,
    revision: ZoneRevision,
}

impl CommittedChild {
    /// Construct a committed child identity from the Resource API response.
    pub fn new(
        resource_ref: ResourceRef,
        owner_ref: ResourceRef,
        zone: ZoneId,
        uid: ResourceUid,
        revision: ZoneRevision,
    ) -> Result<Self, ChildIdentityError> {
        if revision.get() == 0 {
            return Err(ChildIdentityError::InvalidRevision);
        }
        Ok(Self {
            resource_ref,
            owner_ref,
            zone,
            uid,
            revision,
        })
    }

    /// Borrow the returned ResourceRef.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the returned owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the returned Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store-assigned UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the committed Zone revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }
}

impl fmt::Debug for CommittedChild {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommittedChild(<redacted>)")
    }
}

/// Closed failures while binding a CommitBatch response to expected children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitResponseError {
    /// The Resource API returned an error response.
    ApiError,
    /// A returned resource was missing its identity or owner.
    MalformedResponse,
    /// A returned child was absent from the expected set.
    Missing,
    /// A returned child was not expected.
    Extra,
    /// The same ResourceRef appeared more than once.
    Duplicate,
    /// Two expected ResourceRefs were assigned one UID.
    DuplicateUid,
    /// A child owner did not match the Guest owner.
    WrongOwner,
    /// A same-name child used the wrong ResourceType.
    WrongType,
    /// A child response came from another Zone.
    WrongZone,
    /// A returned identity was invalid.
    InvalidIdentity,
}

impl fmt::Display for CommitResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ApiError => "cloud-hypervisor-commit-response-api-error",
            Self::MalformedResponse => "cloud-hypervisor-commit-response-malformed",
            Self::Missing => "cloud-hypervisor-commit-response-missing",
            Self::Extra => "cloud-hypervisor-commit-response-extra",
            Self::Duplicate => "cloud-hypervisor-commit-response-duplicate",
            Self::DuplicateUid => "cloud-hypervisor-commit-response-uid-duplicate",
            Self::WrongOwner => "cloud-hypervisor-commit-response-owner-mismatch",
            Self::WrongType => "cloud-hypervisor-commit-response-type-mismatch",
            Self::WrongZone => "cloud-hypervisor-commit-response-zone-mismatch",
            Self::InvalidIdentity => "cloud-hypervisor-commit-response-identity-invalid",
        })
    }
}

impl std::error::Error for CommitResponseError {}

/// The mapped committed child set keyed by deterministic ResourceRef.
pub type CommittedChildren = BTreeMap<ResourceRef, CommittedChild>;

/// Map every returned child to its expected ResourceRef and committed identity.
pub fn map_commit_response(
    batch: &GuestChildBatch,
    returned: impl IntoIterator<Item = CommittedChild>,
) -> Result<CommittedChildren, CommitResponseError> {
    let expected = batch
        .mutations
        .iter()
        .map(|mutation| (mutation.target.clone(), mutation))
        .collect::<BTreeMap<_, _>>();
    let mut mapped = BTreeMap::new();
    let mut uids = BTreeSet::new();
    for child in returned {
        let Some(expected_mutation) = expected.get(&child.resource_ref) else {
            if expected.keys().any(|reference| {
                reference.name() == child.resource_ref.name()
                    && reference.resource_type() != child.resource_ref.resource_type()
            }) {
                return Err(CommitResponseError::WrongType);
            }
            return Err(CommitResponseError::Extra);
        };
        if mapped.contains_key(&child.resource_ref) {
            return Err(CommitResponseError::Duplicate);
        }
        if !uids.insert(child.uid.clone()) {
            return Err(CommitResponseError::DuplicateUid);
        }
        if child.zone != batch.zone {
            return Err(CommitResponseError::WrongZone);
        }
        if child.owner_ref != batch.owner_ref {
            return Err(CommitResponseError::WrongOwner);
        }
        if expected_mutation.owner_ref != child.owner_ref || expected_mutation.zone != child.zone {
            return Err(CommitResponseError::WrongOwner);
        }
        mapped.insert(child.resource_ref.clone(), child);
    }
    if mapped.len() != expected.len() {
        return Err(CommitResponseError::Missing);
    }
    Ok(mapped)
}

/// Map a generated Resource API CommitBatch response.
pub fn map_wire_commit_response(
    batch: &GuestChildBatch,
    response: &d2b_contracts_resource::resource_proto::CommitBatchResponse,
) -> Result<CommittedChildren, CommitResponseError> {
    if response.error.is_some() {
        return Err(CommitResponseError::ApiError);
    }
    let returned = response
        .resources
        .iter()
        .map(committed_child_from_wire)
        .collect::<Result<Vec<_>, _>>()?;
    map_commit_response(batch, returned)
}

fn committed_child_from_wire(
    resource: &d2b_contracts_resource::resource_proto::ResourceEnvelopeBytes,
) -> Result<CommittedChild, CommitResponseError> {
    let identity = resource
        .identity
        .as_ref()
        .ok_or(CommitResponseError::MalformedResponse)?;
    let resource_ref = ResourceRef::parse(&format!("{}/{}", identity.resource_type, identity.name))
        .map_err(|_| CommitResponseError::InvalidIdentity)?;
    let zone =
        ZoneId::parse(identity.zone.clone()).map_err(|_| CommitResponseError::InvalidIdentity)?;
    let uid = identity
        .uid
        .as_ref()
        .ok_or(CommitResponseError::MalformedResponse)
        .and_then(|value| {
            ResourceUid::parse(value.clone()).map_err(|_| CommitResponseError::InvalidIdentity)
        })?;
    let revision = identity
        .revision
        .filter(|revision| *revision > 0)
        .map(ZoneRevision::new)
        .ok_or(CommitResponseError::MalformedResponse)?;
    let owner_ref = owner_ref_from_canonical_json(&resource.canonical_json)?;
    CommittedChild::new(resource_ref, owner_ref, zone, uid, revision)
        .map_err(|_| CommitResponseError::InvalidIdentity)
}

fn owner_ref_from_canonical_json(bytes: &[u8]) -> Result<ResourceRef, CommitResponseError> {
    let value =
        CanonicalJsonValue::parse(bytes).map_err(|_| CommitResponseError::MalformedResponse)?;
    if value.to_canonical_bytes() != bytes {
        return Err(CommitResponseError::MalformedResponse);
    }
    let root = value
        .as_object()
        .ok_or(CommitResponseError::MalformedResponse)?;
    let metadata = root
        .get("metadata")
        .and_then(CanonicalJsonValue::as_object)
        .ok_or(CommitResponseError::MalformedResponse)?;
    let owner = metadata
        .get("ownerRef")
        .and_then(|value| match value {
            CanonicalJsonValue::String(owner) => Some(owner.as_str()),
            _ => None,
        })
        .ok_or(CommitResponseError::MalformedResponse)?;
    ResourceRef::parse(owner).map_err(|_| CommitResponseError::InvalidIdentity)
}

/// An opaque host-global runtime scope for one Guest incarnation and role.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PrivateRuntimeScope([u8; 32]);

impl fmt::Debug for PrivateRuntimeScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateRuntimeScope(<redacted>)")
    }
}

/// Derive a private runtime scope from immutable Zone/Guest identity.
pub fn derive_private_runtime_scope(
    zone_uid: &ResourceUid,
    guest_uid: &ResourceUid,
    role: &str,
    generation: d2b_contracts_resource::v3::ResourceGeneration,
) -> Result<PrivateRuntimeScope, ChildIdentityError> {
    if !matches!(role, "vmm" | "ch-api" | "guest-control" | "system") {
        return Err(ChildIdentityError::InvalidRuntimeRole);
    }
    let mut digest = Sha256::new();
    digest.update(PRIVATE_RUNTIME_SCOPE_DOMAIN_TAG.as_bytes());
    digest.update([0]);
    for value in [
        zone_uid.as_str().as_bytes(),
        guest_uid.as_str().as_bytes(),
        role.as_bytes(),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value);
    }
    digest.update(generation.get().to_be_bytes());
    Ok(PrivateRuntimeScope(digest.finalize().into()))
}

impl From<GuestSetupDescriptorError> for ChildIdentityError {
    fn from(_: GuestSetupDescriptorError) -> Self {
        Self::DescriptorInvalid
    }
}
