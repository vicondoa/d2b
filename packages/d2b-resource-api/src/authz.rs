//! Native Role and RoleBinding authorization evaluator.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use d2b_contracts::v3::identity::STANDARD_RESOURCE_TYPES;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ControllerGeneration, EvidenceClass, Locality,
    MAX_ROLE_BINDING_SUBJECTS, MAX_ROLE_RULE_EXECUTION_REFS, MAX_ROLE_RULE_RESOURCE_NAMES,
    MAX_ROLE_RULE_RESOURCE_TYPES, MAX_ROLE_RULE_VERBS, MAX_ROLE_RULES, ResourceErrorKind,
    ResourceName, ResourceRef, ResourceTypeName, ResourceUid, ZoneId, ZoneRevision,
};
use d2b_core_controller::rbac::{AuthorizationCacheKey, PolicyRevisionSet, PositiveDecisionCache};
use d2b_resource_store::{
    AdmittedAuthorization, AdmittedAuthorizationTarget, AdmittedVerb, PolicySnapshot,
    StoreMutation, StoreOperationContext,
};
use sha2::{Digest, Sha256};

use crate::admission::{
    AdmissionError, AdmissionIssuer, AdmissionPermit, AdmissionVerifier, AdmittedMutation,
    StoreIdentity, admission_pair,
};

const POSITIVE_CACHE_ENTRIES: usize = 4096;
const POSITIVE_CACHE_TICKS: u64 = 30;
const RESOURCE_SERVICE: &str = "d2b.resource.v3";
const BOOTSTRAP_PURPOSE: &str = "resource-bootstrap";

/// Immutable set of ResourceTypes installed for one API binding.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiCatalog {
    resource_types: BTreeSet<ResourceTypeName>,
}

impl ApiCatalog {
    /// Construct the standard API catalog.
    pub fn standard() -> Self {
        Self {
            resource_types: STANDARD_RESOURCE_TYPES
                .into_iter()
                .map(|value| {
                    ResourceTypeName::parse(value)
                        .expect("the standard ResourceType catalog is validated")
                })
                .collect(),
        }
    }

    /// Extend the standard catalog with installed qualified ResourceTypes.
    pub fn with_extensions(
        extensions: impl IntoIterator<Item = ResourceTypeName>,
    ) -> Result<Self, AuthorizationPolicyError> {
        let mut catalog = Self::standard();
        for resource_type in extensions {
            if !resource_type.as_str().contains(".d2bus.org.")
                || !catalog.resource_types.insert(resource_type)
            {
                return Err(AuthorizationPolicyError::CatalogShape);
            }
        }
        Ok(catalog)
    }

    fn contains(&self, resource_type: &ResourceTypeName) -> bool {
        self.resource_types.contains(resource_type)
    }
}

impl Default for ApiCatalog {
    fn default() -> Self {
        Self::standard()
    }
}

impl core::fmt::Debug for ApiCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ApiCatalog")
            .field("resource_type_count", &self.resource_types.len())
            .finish()
    }
}

/// Resource methods distinguished from their authorization verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApiMethod {
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
    ResolveRef,
    InspectSchema,
    Upgrade,
}

/// Closed resource authorization verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceVerb {
    Get,
    List,
    Watch,
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
    UseCredential,
    AdminCredential,
}

impl ResourceVerb {
    fn admitted(self) -> AdmittedVerb {
        match self {
            Self::Get => AdmittedVerb::Get,
            Self::List => AdmittedVerb::List,
            Self::Watch => AdmittedVerb::Watch,
            Self::Create => AdmittedVerb::Create,
            Self::UpdateSpec => AdmittedVerb::UpdateSpec,
            Self::UpdateStatus => AdmittedVerb::UpdateStatus,
            Self::UpdateMetadata => AdmittedVerb::UpdateMetadata,
            Self::UpdateFinalizers => AdmittedVerb::UpdateFinalizers,
            Self::Delete => AdmittedVerb::Delete,
            Self::UseCredential => AdmittedVerb::UseCredential,
            Self::AdminCredential => AdmittedVerb::AdminCredential,
        }
    }

    fn tag(self) -> u8 {
        self as u8
    }
}

/// Closed ComponentSession authorization verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionVerb {
    Connect,
    Invoke,
    OpenStream,
    Relay,
    Attach,
    Cancel,
    Observe,
    AuditExport,
    SupportBundle,
}

/// One exact target evaluated for a method or atomic batch.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationTarget {
    pub resource_type: ResourceTypeName,
    pub resource_name: Option<ResourceName>,
    pub verb: ResourceVerb,
    pub subresource: Option<String>,
    pub execution_ref: Option<ResourceRef>,
}

/// Immutable method authorization input.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub method: ApiMethod,
    pub zone: ZoneId,
    pub targets: Vec<AuthorizationTarget>,
}

/// Revision and bootstrap state captured from trusted runtime state.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationState {
    pub snapshot: PolicySnapshot,
    pub zone_policy_revision: ZoneRevision,
    pub bootstrap_phase: BootstrapPhase,
    pub now_tick: u64,
}

/// Durable bootstrap-policy phase.
#[derive(Clone, PartialEq, Eq)]
pub enum BootstrapPhase {
    Unprovisioned {
        zone: ZoneId,
        controller_generation: ControllerGeneration,
        provider_generation: d2b_contracts::v3::ResourceGeneration,
    },
    Provisioned {
        zone: ZoneId,
        system_core_uid: ResourceUid,
        system_minijail_uid: ResourceUid,
        controller_generation: ControllerGeneration,
        provider_generation: d2b_contracts::v3::ResourceGeneration,
    },
    Disabled,
}

/// Exact subject binding compiled from one RoleBinding.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoundSubject {
    pub subject_ref: ResourceRef,
    pub subject_uid: ResourceUid,
}

/// Optional narrowing applied by a RoleBinding.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct BindingScope {
    pub zones: BTreeSet<ZoneId>,
    pub resource_names: BTreeSet<ResourceName>,
    pub execution_refs: BTreeSet<ResourceRef>,
}

/// Authority that created a relay-bearing binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayGrantAuthority {
    None,
    CoreGenerated,
    DurableLocalAdmin,
}

/// Validated evaluator projection of one Role rule.
#[derive(Clone, PartialEq, Eq)]
pub struct PolicyRule {
    resource_types: BTreeSet<ResourceTypeName>,
    resource_verbs: BTreeSet<ResourceVerb>,
    session_verbs: BTreeSet<SessionVerb>,
    subresources: BTreeSet<String>,
    resource_names: BTreeSet<ResourceName>,
    zones: BTreeSet<ZoneId>,
    execution_refs: BTreeSet<ResourceRef>,
}

impl core::fmt::Debug for AuthorizationTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthorizationTarget")
            .field("verb", &self.verb)
            .field("resource_type", &"<redacted>")
            .field("has_resource_name", &self.resource_name.is_some())
            .field("has_subresource", &self.subresource.is_some())
            .field("has_execution_ref", &self.execution_ref.is_some())
            .finish()
    }
}

impl core::fmt::Debug for AuthorizationRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthorizationRequest")
            .field("method", &self.method)
            .field("zone", &"<redacted>")
            .field("target_count", &self.targets.len())
            .finish()
    }
}

impl core::fmt::Debug for AuthorizationState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AuthorizationState")
            .field("snapshot", &"<redacted>")
            .field("zone_policy_revision", &"<redacted>")
            .field("bootstrap_phase", &self.bootstrap_phase)
            .field("now_tick", &"<redacted>")
            .finish()
    }
}

impl core::fmt::Debug for BootstrapPhase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Unprovisioned { .. } => "BootstrapPhase::Unprovisioned(<redacted>)",
            Self::Provisioned { .. } => "BootstrapPhase::Provisioned(<redacted>)",
            Self::Disabled => "BootstrapPhase::Disabled",
        })
    }
}

impl core::fmt::Debug for BoundSubject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BoundSubject(<redacted>)")
    }
}

impl core::fmt::Debug for BindingScope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BindingScope")
            .field("zone_count", &self.zones.len())
            .field("resource_name_count", &self.resource_names.len())
            .field("execution_ref_count", &self.execution_refs.len())
            .finish()
    }
}

impl core::fmt::Debug for PolicyRule {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PolicyRule")
            .field("resource_type_count", &self.resource_types.len())
            .field("resource_verb_count", &self.resource_verbs.len())
            .field("session_verb_count", &self.session_verbs.len())
            .field("subresource_count", &self.subresources.len())
            .field("resource_name_count", &self.resource_names.len())
            .field("zone_count", &self.zones.len())
            .field("execution_ref_count", &self.execution_refs.len())
            .finish()
    }
}

impl PolicyRule {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        catalog: &ApiCatalog,
        resource_types: impl IntoIterator<Item = ResourceTypeName>,
        resource_verbs: impl IntoIterator<Item = ResourceVerb>,
        session_verbs: impl IntoIterator<Item = SessionVerb>,
        subresources: impl IntoIterator<Item = String>,
        resource_names: impl IntoIterator<Item = ResourceName>,
        zones: impl IntoIterator<Item = ZoneId>,
        execution_refs: impl IntoIterator<Item = ResourceRef>,
    ) -> Result<Self, AuthorizationPolicyError> {
        let rule = Self {
            resource_types: resource_types.into_iter().collect(),
            resource_verbs: resource_verbs.into_iter().collect(),
            session_verbs: session_verbs.into_iter().collect(),
            subresources: subresources.into_iter().collect(),
            resource_names: resource_names.into_iter().collect(),
            zones: zones.into_iter().collect(),
            execution_refs: execution_refs.into_iter().collect(),
        };
        if rule.resource_types.len() > MAX_ROLE_RULE_RESOURCE_TYPES
            || rule.resource_verbs.len() + rule.session_verbs.len() > MAX_ROLE_RULE_VERBS
            || rule.resource_names.len() > MAX_ROLE_RULE_RESOURCE_NAMES
            || rule.execution_refs.len() > MAX_ROLE_RULE_EXECUTION_REFS
            || rule
                .subresources
                .iter()
                .any(|value| value.is_empty() || value.len() > 128 || !value.is_ascii())
        {
            return Err(AuthorizationPolicyError::RuleBounds);
        }
        if rule
            .resource_types
            .iter()
            .any(|resource_type| !catalog.contains(resource_type))
        {
            return Err(AuthorizationPolicyError::UnknownResourceType);
        }
        if rule.resource_verbs.contains(&ResourceVerb::UseCredential)
            && (rule.resource_types.len() != 1
                || !rule
                    .resource_types
                    .iter()
                    .any(|resource_type| resource_type.as_str() == "Credential")
                || rule.subresources.is_empty())
        {
            return Err(AuthorizationPolicyError::CredentialScope);
        }
        if rule.resource_verbs.contains(&ResourceVerb::AdminCredential)
            && (rule.resource_types.len() != 1
                || !rule
                    .resource_types
                    .iter()
                    .any(|resource_type| resource_type.as_str() == "Credential")
                || rule.subresources.is_empty()
                || rule.subresources.iter().any(|subresource| {
                    !matches!(subresource.as_str(), "create" | "update-spec" | "delete")
                })
                || rule.subresources.iter().any(|subresource| {
                    let required = match subresource.as_str() {
                        "create" => ResourceVerb::Create,
                        "update-spec" => ResourceVerb::UpdateSpec,
                        "delete" => ResourceVerb::Delete,
                        _ => return true,
                    };
                    !rule.resource_verbs.contains(&required)
                }))
        {
            return Err(AuthorizationPolicyError::CredentialScope);
        }
        Ok(rule)
    }

    fn permits_target(&self, target: &AuthorizationTarget, zone: &ZoneId) -> bool {
        self.resource_types.contains(&target.resource_type)
            && self.resource_verbs.contains(&target.verb)
            && (self.zones.is_empty() || self.zones.contains(zone))
            && (self.resource_names.is_empty()
                || target
                    .resource_name
                    .as_ref()
                    .is_some_and(|name| self.resource_names.contains(name)))
            && (self.subresources.is_empty()
                || target
                    .subresource
                    .as_ref()
                    .is_some_and(|value| self.subresources.contains(value)))
            && (self.execution_refs.is_empty()
                || target
                    .execution_ref
                    .as_ref()
                    .is_some_and(|value| self.execution_refs.contains(value)))
    }
}

/// Validated evaluator projection of one Role.
#[derive(Clone, PartialEq, Eq)]
pub struct CompiledRole {
    pub role_ref: ResourceRef,
    pub rules: Vec<PolicyRule>,
}

impl core::fmt::Debug for CompiledRole {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CompiledRole")
            .field("role_ref", &"<redacted>")
            .field("rule_count", &self.rules.len())
            .finish()
    }
}

impl CompiledRole {
    pub fn new(
        role_ref: ResourceRef,
        rules: Vec<PolicyRule>,
    ) -> Result<Self, AuthorizationPolicyError> {
        if role_ref.resource_type().as_str() != "Role" || rules.len() > MAX_ROLE_RULES {
            return Err(AuthorizationPolicyError::RoleShape);
        }
        Ok(Self { role_ref, rules })
    }
}

/// Validated evaluator projection of one RoleBinding.
#[derive(Clone, PartialEq, Eq)]
pub struct CompiledRoleBinding {
    pub role_ref: ResourceRef,
    pub subjects: BTreeSet<BoundSubject>,
    pub scope: BindingScope,
    pub relay_authority: RelayGrantAuthority,
}

impl core::fmt::Debug for CompiledRoleBinding {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CompiledRoleBinding")
            .field("role_ref", &"<redacted>")
            .field("subject_count", &self.subjects.len())
            .field("scope", &self.scope)
            .field("relay_authority", &self.relay_authority)
            .finish()
    }
}

impl CompiledRoleBinding {
    pub fn new(
        role_ref: ResourceRef,
        subjects: impl IntoIterator<Item = BoundSubject>,
        scope: BindingScope,
        relay_authority: RelayGrantAuthority,
    ) -> Result<Self, AuthorizationPolicyError> {
        let subjects = subjects.into_iter().collect::<Vec<_>>();
        let subject_count = subjects.len();
        let subjects = subjects.into_iter().collect::<BTreeSet<_>>();
        if role_ref.resource_type().as_str() != "Role"
            || subjects.is_empty()
            || subjects.len() != subject_count
            || subjects.len() > MAX_ROLE_BINDING_SUBJECTS
            || subjects.iter().any(|subject| {
                !matches!(
                    subject.subject_ref.resource_type().as_str(),
                    "Zone" | "ZoneLink" | "User" | "Provider" | "Host" | "Guest" | "Process"
                )
            })
            || scope.resource_names.len() > MAX_ROLE_RULE_RESOURCE_NAMES
            || scope.execution_refs.len() > MAX_ROLE_RULE_EXECUTION_REFS
        {
            return Err(AuthorizationPolicyError::BindingShape);
        }
        Ok(Self {
            role_ref,
            subjects,
            scope,
            relay_authority,
        })
    }

    fn contains_subject(&self, context: &AuthenticatedSubjectContext) -> bool {
        self.subjects.contains(&BoundSubject {
            subject_ref: context.subject_ref().clone(),
            subject_uid: context.subject_uid().clone(),
        })
    }

    fn permits_scope(&self, target: &AuthorizationTarget, zone: &ZoneId) -> bool {
        (self.scope.zones.is_empty() || self.scope.zones.contains(zone))
            && (self.scope.resource_names.is_empty()
                || target
                    .resource_name
                    .as_ref()
                    .is_some_and(|name| self.scope.resource_names.contains(name)))
            && (self.scope.execution_refs.is_empty()
                || target
                    .execution_ref
                    .as_ref()
                    .is_some_and(|reference| self.scope.execution_refs.contains(reference)))
    }
}

/// One immutable installed policy revision.
#[derive(Clone, PartialEq, Eq)]
pub struct PolicySet {
    pub policy_revision: u64,
    catalog: ApiCatalog,
    roles: BTreeMap<ResourceRef, CompiledRole>,
    bindings: Vec<CompiledRoleBinding>,
}

impl core::fmt::Debug for PolicySet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PolicySet")
            .field("policy_revision", &"<redacted>")
            .field("catalog", &self.catalog)
            .field("role_count", &self.roles.len())
            .field("binding_count", &self.bindings.len())
            .finish()
    }
}

impl PolicySet {
    pub fn new(
        catalog: &ApiCatalog,
        policy_revision: u64,
        roles: Vec<CompiledRole>,
        bindings: Vec<CompiledRoleBinding>,
    ) -> Result<Self, AuthorizationPolicyError> {
        if policy_revision == 0 {
            return Err(AuthorizationPolicyError::PolicyRevisionZero);
        }
        let role_count = roles.len();
        let roles = roles
            .into_iter()
            .map(|role| (role.role_ref.clone(), role))
            .collect::<BTreeMap<_, _>>();
        if roles.len() != role_count {
            return Err(AuthorizationPolicyError::DuplicateRole);
        }
        for binding in &bindings {
            let role = roles
                .get(&binding.role_ref)
                .ok_or(AuthorizationPolicyError::MissingRole)?;
            let has_relay = role
                .rules
                .iter()
                .any(|rule| rule.session_verbs.contains(&SessionVerb::Relay));
            if has_relay && binding.relay_authority == RelayGrantAuthority::None {
                return Err(AuthorizationPolicyError::RelayGrantRestricted);
            }
        }
        Ok(Self {
            policy_revision,
            catalog: catalog.clone(),
            roles,
            bindings,
        })
    }
}

/// Positive exact capabilities, never inferred from a denial.
#[derive(Clone, PartialEq, Eq)]
pub struct PositiveCapabilities {
    pub resources: Vec<AuthorizationTarget>,
    pub session_verbs: BTreeSet<SessionVerb>,
}

impl core::fmt::Debug for PositiveCapabilities {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PositiveCapabilities")
            .field("resource_count", &self.resources.len())
            .field("session_verb_count", &self.session_verbs.len())
            .finish()
    }
}

/// Typed fail-closed authorization outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationDenial {
    PolicyUnavailable,
    PolicyRevisionChanged,
    ZoneMismatch,
    NoMatchingGrant,
    RelayOriginInvalid,
    RelayGrantMissing,
    RelayTargetGrantMissing,
    BootstrapDenied,
    UnknownResourceType,
}

impl AuthorizationDenial {
    pub const fn resource_error_kind(self) -> ResourceErrorKind {
        match self {
            Self::RelayOriginInvalid | Self::RelayGrantMissing => ResourceErrorKind::RelayDenied,
            Self::RelayTargetGrantMissing => ResourceErrorKind::AuthorizationDenied,
            _ => ResourceErrorKind::AuthorizationDenied,
        }
    }
}

/// Successful authorization evidence returned to the service.
pub struct AuthorizationGrant {
    permit: AdmissionPermit,
}

impl core::fmt::Debug for AuthorizationGrant {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AuthorizationGrant(<redacted>)")
    }
}

impl AuthorizationGrant {
    pub(crate) fn admit(
        self,
        mutations: Vec<StoreMutation>,
        operation: StoreOperationContext,
    ) -> Result<AdmittedMutation, AdmissionError> {
        self.permit.admit(mutations, operation)
    }
}

/// Single native evaluator and positive-decision cache.
pub struct NativeAuthorizer {
    catalog: ApiCatalog,
    policy: RwLock<Option<Arc<PolicySet>>>,
    cache: PositiveDecisionCache,
    admission: AdmissionIssuer,
}

impl core::fmt::Debug for NativeAuthorizer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("NativeAuthorizer(<redacted>)")
    }
}

impl NativeAuthorizer {
    /// Build the evaluator and the verifier transferred to its store backend.
    pub fn new(
        catalog: ApiCatalog,
        policy: Option<PolicySet>,
    ) -> Result<(Self, AdmissionVerifier, StoreIdentity), AuthorizationPolicyError> {
        let (admission, verifier, store_identity) = admission_pair();
        let authorizer = Self::from_issuer(catalog, policy, admission)?;
        Ok((authorizer, verifier, store_identity))
    }

    fn from_issuer(
        catalog: ApiCatalog,
        policy: Option<PolicySet>,
        admission: AdmissionIssuer,
    ) -> Result<Self, AuthorizationPolicyError> {
        if policy
            .as_ref()
            .is_some_and(|policy| policy.catalog != catalog)
        {
            return Err(AuthorizationPolicyError::CatalogMismatch);
        }
        Ok(Self {
            catalog,
            policy: RwLock::new(policy.map(Arc::new)),
            cache: PositiveDecisionCache::new(POSITIVE_CACHE_ENTRIES),
            admission,
        })
    }

    pub fn replace_policy(
        &self,
        policy: PolicySet,
        state: &AuthorizationState,
    ) -> Result<(), AuthorizationPolicyError> {
        if policy.catalog != self.catalog {
            return Err(AuthorizationPolicyError::CatalogMismatch);
        }
        if policy.policy_revision != state.snapshot.policy_revision {
            return Err(AuthorizationPolicyError::PolicyStateRevisionMismatch);
        }
        let mut installed = self.write_policy();
        *installed = Some(Arc::new(policy));
        self.cache.clear();
        Ok(())
    }

    pub fn mark_policy_unavailable(&self) {
        *self.write_policy() = None;
        self.cache.clear();
    }

    pub fn authorize(
        &self,
        context: &AuthenticatedSubjectContext,
        request: &AuthorizationRequest,
        state: &AuthorizationState,
    ) -> Result<AuthorizationGrant, AuthorizationDenial> {
        self.authorize_before_grant(context, request, state, || {})
    }

    fn authorize_before_grant(
        &self,
        context: &AuthenticatedSubjectContext,
        request: &AuthorizationRequest,
        state: &AuthorizationState,
        before_grant: impl FnOnce(),
    ) -> Result<AuthorizationGrant, AuthorizationDenial> {
        if request.targets.is_empty() {
            return Err(AuthorizationDenial::NoMatchingGrant);
        }
        if request
            .targets
            .iter()
            .any(|target| !self.catalog.contains(&target.resource_type))
        {
            return Err(AuthorizationDenial::UnknownResourceType);
        }
        if context.zone_ref().resource_type().as_str() != "Zone"
            || context.zone_ref().name().as_str() != request.zone.as_str()
        {
            return Err(AuthorizationDenial::ZoneMismatch);
        }
        let relay_hop = authenticated_relay_hop(context)?;
        if state.snapshot.policy_revision == 0 {
            return authorize_bootstrap(&self.admission, context, request, state, relay_hop);
        }

        let installed = self.read_policy();
        let policy = installed
            .as_ref()
            .ok_or(AuthorizationDenial::PolicyUnavailable)?;
        if policy.policy_revision != state.snapshot.policy_revision {
            return Err(AuthorizationDenial::PolicyRevisionChanged);
        }

        let cache_key = cache_key(context, request, relay_hop);
        let revisions = revision_set(state);
        if !self.cache.contains(&cache_key, revisions, state.now_tick) {
            evaluate_policy(policy, context, request, relay_hop)?;
            self.cache.insert_allow(
                cache_key,
                revisions,
                state.now_tick.saturating_add(POSITIVE_CACHE_TICKS),
                state.now_tick,
            );
        }
        before_grant();
        Ok(grant(&self.admission, context, request, state.snapshot))
    }

    pub fn positive_capabilities(
        &self,
        context: &AuthenticatedSubjectContext,
        zone: &ZoneId,
        state: &AuthorizationState,
    ) -> Result<PositiveCapabilities, AuthorizationDenial> {
        let policy = self
            .read_policy()
            .clone()
            .ok_or(AuthorizationDenial::PolicyUnavailable)?;
        if policy.policy_revision != state.snapshot.policy_revision {
            return Err(AuthorizationDenial::PolicyRevisionChanged);
        }
        let mut resources = Vec::new();
        let mut session_verbs = BTreeSet::new();
        for binding in policy
            .bindings
            .iter()
            .filter(|binding| binding.contains_subject(context))
            .filter(|binding| binding.scope.zones.is_empty() || binding.scope.zones.contains(zone))
        {
            let Some(role) = policy.roles.get(&binding.role_ref) else {
                continue;
            };
            for rule in &role.rules {
                if !rule.zones.is_empty() && !rule.zones.contains(zone) {
                    continue;
                }
                session_verbs.extend(rule.session_verbs.iter().copied());
                for resource_type in &rule.resource_types {
                    for verb in &rule.resource_verbs {
                        if rule.resource_names.is_empty() {
                            let target = AuthorizationTarget {
                                resource_type: resource_type.clone(),
                                resource_name: None,
                                verb: *verb,
                                subresource: None,
                                execution_ref: None,
                            };
                            if binding.permits_scope(&target, zone) && !resources.contains(&target)
                            {
                                resources.push(target);
                            }
                        } else {
                            for name in &rule.resource_names {
                                let target = AuthorizationTarget {
                                    resource_type: resource_type.clone(),
                                    resource_name: Some(name.clone()),
                                    verb: *verb,
                                    subresource: None,
                                    execution_ref: None,
                                };
                                if binding.permits_scope(&target, zone)
                                    && !resources.contains(&target)
                                {
                                    resources.push(target);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(PositiveCapabilities {
            resources,
            session_verbs,
        })
    }

    fn read_policy(&self) -> RwLockReadGuard<'_, Option<Arc<PolicySet>>> {
        self.policy
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_policy(&self) -> RwLockWriteGuard<'_, Option<Arc<PolicySet>>> {
        self.policy
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) fn authenticated_relay_hop(
    context: &AuthenticatedSubjectContext,
) -> Result<bool, AuthorizationDenial> {
    match context.transport_binding().locality() {
        Locality::Local => Ok(false),
        Locality::AdjacentZone
            if context.evidence_class() == EvidenceClass::EnrolledKk
                && matches!(
                    context.subject_ref().resource_type().as_str(),
                    "Zone" | "ZoneLink"
                ) =>
        {
            Ok(true)
        }
        Locality::AdjacentZone | Locality::Remote => Err(AuthorizationDenial::RelayOriginInvalid),
    }
}

fn evaluate_policy(
    policy: &PolicySet,
    context: &AuthenticatedSubjectContext,
    request: &AuthorizationRequest,
    relay_hop: bool,
) -> Result<(), AuthorizationDenial> {
    if relay_hop {
        let relay_allowed = policy
            .bindings
            .iter()
            .filter(|binding| {
                binding.contains_subject(context)
                    && binding.relay_authority != RelayGrantAuthority::None
            })
            .filter_map(|binding| policy.roles.get(&binding.role_ref))
            .flat_map(|role| &role.rules)
            .any(|rule| rule.session_verbs.contains(&SessionVerb::Relay));
        if !relay_allowed {
            return Err(AuthorizationDenial::RelayGrantMissing);
        }
    }

    for target in &request.targets {
        let allowed = policy
            .bindings
            .iter()
            .filter(|binding| {
                binding.contains_subject(context) && binding.permits_scope(target, &request.zone)
            })
            .filter_map(|binding| policy.roles.get(&binding.role_ref))
            .flat_map(|role| &role.rules)
            .any(|rule| rule.permits_target(target, &request.zone));
        if !allowed {
            return Err(if relay_hop {
                AuthorizationDenial::RelayTargetGrantMissing
            } else {
                AuthorizationDenial::NoMatchingGrant
            });
        }
    }
    Ok(())
}

fn grant(
    admission: &AdmissionIssuer,
    context: &AuthenticatedSubjectContext,
    request: &AuthorizationRequest,
    policy_snapshot: PolicySnapshot,
) -> AuthorizationGrant {
    AuthorizationGrant {
        permit: admission.record_allow(
            AdmittedAuthorization {
                zone: request.zone.clone(),
                subject_ref: context.subject_ref().clone(),
                subject_uid: context.subject_uid().clone(),
                targets: request
                    .targets
                    .iter()
                    .map(|target| AdmittedAuthorizationTarget {
                        resource_type: target.resource_type.clone(),
                        resource_name: target.resource_name.clone(),
                        verb: target.verb.admitted(),
                        subresource: target.subresource.clone(),
                        execution_ref: target.execution_ref.clone(),
                    })
                    .collect(),
            },
            policy_snapshot,
        ),
    }
}

fn cache_key(
    context: &AuthenticatedSubjectContext,
    request: &AuthorizationRequest,
    relay_hop: bool,
) -> AuthorizationCacheKey {
    let mut digest = Sha256::new();
    digest.update([request.method as u8]);
    digest.update(request.zone.as_str().as_bytes());
    digest.update([u8::from(relay_hop)]);
    digest.update([evidence_tag(context.evidence_class())]);
    digest.update([locality_tag(context.transport_binding().locality())]);
    digest.update(context.session_purpose().as_str().as_bytes());
    digest.update([0]);
    digest.update(context.service().as_str().as_bytes());
    if let Some(generation) = context.controller_generation() {
        digest.update([1]);
        digest.update(generation.get().to_be_bytes());
    } else {
        digest.update([0]);
    }
    if let Some(generation) = context.provider_generation() {
        digest.update([1]);
        digest.update(generation.get().to_be_bytes());
    } else {
        digest.update([0]);
    }
    for target in &request.targets {
        digest.update(target.resource_type.as_str().as_bytes());
        if let Some(name) = &target.resource_name {
            digest.update([0]);
            digest.update(name.as_str().as_bytes());
        }
        digest.update([target.verb.tag()]);
        if let Some(subresource) = &target.subresource {
            digest.update([0]);
            digest.update(subresource.as_bytes());
        }
        if let Some(execution_ref) = &target.execution_ref {
            digest.update([0]);
            digest.update(execution_ref.to_string().as_bytes());
        }
    }
    AuthorizationCacheKey::new(
        context.subject_ref().clone(),
        context.subject_uid().clone(),
        digest.finalize().into(),
    )
}

const fn evidence_tag(value: EvidenceClass) -> u8 {
    match value {
        EvidenceClass::UnixPeer => 1,
        EvidenceClass::EnrolledKk => 2,
        EvidenceClass::BootstrapIkpsk2 => 3,
        EvidenceClass::NativeVsock => 4,
    }
}

const fn locality_tag(value: Locality) -> u8 {
    match value {
        Locality::Local => 1,
        Locality::AdjacentZone => 2,
        Locality::Remote => 3,
    }
}

fn revision_set(state: &AuthorizationState) -> PolicyRevisionSet {
    PolicyRevisionSet {
        policy_revision: state.snapshot.policy_revision,
        api_catalog_revision: state.snapshot.api_catalog_revision,
        active_configuration_revision: state.snapshot.active_configuration_revision,
        zone_policy_revision: state.zone_policy_revision,
    }
}

#[derive(Debug, Clone, Copy)]
struct BootstrapRow {
    subject_name: &'static str,
    method: ApiMethod,
    resource_type: &'static str,
    verb: ResourceVerb,
}

const fn bootstrap_row(
    subject_name: &'static str,
    method: ApiMethod,
    resource_type: &'static str,
    verb: ResourceVerb,
) -> BootstrapRow {
    BootstrapRow {
        subject_name,
        method,
        resource_type,
        verb,
    }
}

const BOOTSTRAP_ROWS: &[BootstrapRow; 42] = &[
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "Zone",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "Provider",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "Host",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "User",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "Role",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "RoleBinding",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::Create,
        "Process",
        ResourceVerb::Create,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::UpdateStatus,
        "Zone",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::UpdateStatus,
        "Provider",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::UpdateStatus,
        "Host",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::UpdateStatus,
        "User",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::UpdateStatus,
        "Process",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row("system-core", ApiMethod::Get, "Zone", ResourceVerb::Get),
    bootstrap_row("system-core", ApiMethod::Get, "Provider", ResourceVerb::Get),
    bootstrap_row("system-core", ApiMethod::Get, "Host", ResourceVerb::Get),
    bootstrap_row("system-core", ApiMethod::Get, "User", ResourceVerb::Get),
    bootstrap_row("system-core", ApiMethod::Get, "Process", ResourceVerb::Get),
    bootstrap_row("system-core", ApiMethod::List, "Zone", ResourceVerb::List),
    bootstrap_row(
        "system-core",
        ApiMethod::List,
        "Provider",
        ResourceVerb::List,
    ),
    bootstrap_row("system-core", ApiMethod::List, "Host", ResourceVerb::List),
    bootstrap_row("system-core", ApiMethod::List, "User", ResourceVerb::List),
    bootstrap_row(
        "system-core",
        ApiMethod::List,
        "Process",
        ResourceVerb::List,
    ),
    bootstrap_row("system-core", ApiMethod::Watch, "Zone", ResourceVerb::Watch),
    bootstrap_row(
        "system-core",
        ApiMethod::Watch,
        "Provider",
        ResourceVerb::Watch,
    ),
    bootstrap_row("system-core", ApiMethod::Watch, "Host", ResourceVerb::Watch),
    bootstrap_row("system-core", ApiMethod::Watch, "User", ResourceVerb::Watch),
    bootstrap_row(
        "system-core",
        ApiMethod::Watch,
        "Process",
        ResourceVerb::Watch,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::ResolveRef,
        "Zone",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::ResolveRef,
        "Provider",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::ResolveRef,
        "Host",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::ResolveRef,
        "User",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::ResolveRef,
        "Process",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-core",
        ApiMethod::InspectSchema,
        "Provider",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::Get,
        "Process",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::Get,
        "EphemeralProcess",
        ResourceVerb::Get,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::List,
        "Process",
        ResourceVerb::List,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::List,
        "EphemeralProcess",
        ResourceVerb::List,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::Watch,
        "Process",
        ResourceVerb::Watch,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::Watch,
        "EphemeralProcess",
        ResourceVerb::Watch,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::UpdateStatus,
        "Process",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::UpdateStatus,
        "EphemeralProcess",
        ResourceVerb::UpdateStatus,
    ),
    bootstrap_row(
        "system-minijail",
        ApiMethod::InspectSchema,
        "Process",
        ResourceVerb::Get,
    ),
];

fn authorize_bootstrap(
    admission: &AdmissionIssuer,
    context: &AuthenticatedSubjectContext,
    request: &AuthorizationRequest,
    state: &AuthorizationState,
    relay_hop: bool,
) -> Result<AuthorizationGrant, AuthorizationDenial> {
    let (zone, core_uid, minijail_uid, controller_generation, provider_generation, unprovisioned) =
        match &state.bootstrap_phase {
            BootstrapPhase::Unprovisioned {
                zone,
                controller_generation,
                provider_generation,
            } => (
                zone,
                None,
                None,
                *controller_generation,
                *provider_generation,
                true,
            ),
            BootstrapPhase::Provisioned {
                zone,
                system_core_uid,
                system_minijail_uid,
                controller_generation,
                provider_generation,
            } => (
                zone,
                Some(system_core_uid),
                Some(system_minijail_uid),
                *controller_generation,
                *provider_generation,
                false,
            ),
            BootstrapPhase::Disabled => return Err(AuthorizationDenial::BootstrapDenied),
        };
    if relay_hop
        || &request.zone != zone
        || context.evidence_class() != EvidenceClass::UnixPeer
        || context.transport_binding().locality() != Locality::Local
        || context.session_purpose().as_str() != BOOTSTRAP_PURPOSE
        || context.service().as_str() != RESOURCE_SERVICE
        || context.controller_generation() != Some(controller_generation)
        || context.provider_generation() != Some(provider_generation)
        || context.subject_ref().resource_type().as_str() != "Provider"
    {
        return Err(AuthorizationDenial::BootstrapDenied);
    }
    let subject_name = context.subject_ref().name().as_str();
    let zone_name =
        ResourceName::parse(zone.as_str()).map_err(|_| AuthorizationDenial::BootstrapDenied)?;
    if !unprovisioned {
        let expected_uid = match subject_name {
            "system-core" => core_uid,
            "system-minijail" => minijail_uid,
            _ => None,
        };
        if expected_uid != Some(context.subject_uid()) {
            return Err(AuthorizationDenial::BootstrapDenied);
        }
    }
    for target in &request.targets {
        let allowed = BOOTSTRAP_ROWS.iter().any(|row| {
            row.subject_name == subject_name
                && row.method == request.method
                && row.resource_type == target.resource_type.as_str()
                && row.verb == target.verb
        });
        if !allowed {
            return Err(AuthorizationDenial::BootstrapDenied);
        }
        let compiled_name = match target.resource_type.as_str() {
            "Zone" => target
                .resource_name
                .as_ref()
                .is_none_or(|name| name == &zone_name),
            "Provider" => target
                .resource_name
                .as_ref()
                .is_none_or(|name| matches!(name.as_str(), "system-core" | "system-minijail")),
            _ => true,
        };
        let unprovisioned_create = !unprovisioned
            || (request.method == ApiMethod::Create
                && matches!(target.resource_type.as_str(), "Zone" | "Provider"));
        if !compiled_name || !unprovisioned_create {
            return Err(AuthorizationDenial::BootstrapDenied);
        }
    }
    Ok(grant(admission, context, request, state.snapshot))
}

/// Invalid compiled policy projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationPolicyError {
    RuleBounds,
    UnknownResourceType,
    CatalogShape,
    CatalogMismatch,
    CredentialScope,
    RoleShape,
    BindingShape,
    MissingRole,
    DuplicateRole,
    RelayGrantRestricted,
    PolicyRevisionZero,
    PolicyStateRevisionMismatch,
}

impl core::fmt::Display for AuthorizationPolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::RuleBounds => "Role rule exceeds a frozen bound",
            Self::UnknownResourceType => "Role rule names an uninstalled ResourceType",
            Self::CatalogShape => "API catalog extension set is invalid",
            Self::CatalogMismatch => "policy was compiled for a different API catalog",
            Self::CredentialScope => "Credential verb requires an exact Credential subresource",
            Self::RoleShape => "Role evaluator projection is invalid",
            Self::BindingShape => "RoleBinding evaluator projection is invalid",
            Self::MissingRole => "RoleBinding references a missing Role",
            Self::DuplicateRole => "policy contains duplicate Role identities",
            Self::RelayGrantRestricted => "relay grant is not core or durable-admin authorized",
            Self::PolicyRevisionZero => "stored policy revision must be nonzero",
            Self::PolicyStateRevisionMismatch => {
                "installed policy revision does not match trusted runtime state"
            }
        })
    }
}

impl std::error::Error for AuthorizationPolicyError {}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{
        BindingDigest, ConfigurationGeneration, ReconnectGeneration, ResourceGeneration,
        SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose, TranscriptHash,
        TransportBinding,
    };

    fn test_issuer() -> AdmissionIssuer {
        crate::admission::admission_pair().0
    }

    fn subject(
        locality: Locality,
        evidence: EvidenceClass,
        subject_ref: &str,
    ) -> AuthenticatedSubjectContext {
        AuthenticatedSubjectContext::new(
            ResourceRef::parse(subject_ref).unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
            evidence,
            SessionPurpose::parse("resource-api").unwrap(),
            ServiceName::parse(RESOURCE_SERVICE).unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                TransportBinding::new(
                    locality,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
    }

    fn state(revision: u64) -> AuthorizationState {
        AuthorizationState {
            snapshot: PolicySnapshot {
                policy_revision: revision,
                api_catalog_revision: 2,
                active_configuration_revision: ConfigurationGeneration::new(3).unwrap(),
                controller_generation: None,
            },
            zone_policy_revision: ZoneRevision::new(revision),
            bootstrap_phase: BootstrapPhase::Disabled,
            now_tick: 1,
        }
    }

    fn bootstrap_subject(subject_name: &str, subject_uid: &str) -> AuthenticatedSubjectContext {
        AuthenticatedSubjectContext::new(
            ResourceRef::parse(&format!("Provider/{subject_name}")).unwrap(),
            ResourceUid::parse(subject_uid).unwrap(),
            ResourceRef::parse("Zone/dev").unwrap(),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse(BOOTSTRAP_PURPOSE).unwrap(),
            ServiceName::parse(RESOURCE_SERVICE).unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                TransportBinding::new(
                    Locality::Local,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
        .with_controller_generation(ControllerGeneration::new(11).unwrap())
        .with_provider_generation(ResourceGeneration::new(12).unwrap())
    }

    fn bootstrap_state(phase: BootstrapPhase) -> AuthorizationState {
        AuthorizationState {
            snapshot: PolicySnapshot {
                policy_revision: 0,
                api_catalog_revision: 1,
                active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                controller_generation: Some(ControllerGeneration::new(11).unwrap()),
            },
            zone_policy_revision: ZoneRevision::new(0),
            bootstrap_phase: phase,
            now_tick: 1,
        }
    }

    fn bootstrap_target(resource_type: &str, verb: ResourceVerb) -> AuthorizationTarget {
        let resource_name = match resource_type {
            "Zone" => "dev",
            "Provider" => "system-core",
            _ => "app",
        };
        AuthorizationTarget {
            resource_type: ResourceTypeName::parse(resource_type).unwrap(),
            resource_name: Some(ResourceName::parse(resource_name).unwrap()),
            verb,
            subresource: None,
            execution_ref: None,
        }
    }

    fn target(verb: ResourceVerb) -> AuthorizationTarget {
        AuthorizationTarget {
            resource_type: ResourceTypeName::parse("Process").unwrap(),
            resource_name: Some(ResourceName::parse("app").unwrap()),
            verb,
            subresource: None,
            execution_ref: None,
        }
    }

    fn policy(
        revision: u64,
        context: &AuthenticatedSubjectContext,
        target_verb: Option<ResourceVerb>,
        relay: bool,
    ) -> PolicySet {
        let catalog = ApiCatalog::standard();
        let mut rules = Vec::new();
        if let Some(verb) = target_verb {
            rules.push(
                PolicyRule::new(
                    &catalog,
                    [ResourceTypeName::parse("Process").unwrap()],
                    [verb],
                    [],
                    [],
                    [ResourceName::parse("app").unwrap()],
                    [ZoneId::parse("dev").unwrap()],
                    [],
                )
                .unwrap(),
            );
        }
        if relay {
            rules.push(
                PolicyRule::new(&catalog, [], [], [SessionVerb::Relay], [], [], [], []).unwrap(),
            );
        }
        let role = CompiledRole::new(ResourceRef::parse("Role/operator").unwrap(), rules).unwrap();
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [BoundSubject {
                subject_ref: context.subject_ref().clone(),
                subject_uid: context.subject_uid().clone(),
            }],
            BindingScope::default(),
            if relay {
                RelayGrantAuthority::CoreGenerated
            } else {
                RelayGrantAuthority::None
            },
        )
        .unwrap();
        PolicySet::new(&catalog, revision, vec![role], vec![binding]).unwrap()
    }

    fn request() -> AuthorizationRequest {
        AuthorizationRequest {
            method: ApiMethod::Get,
            zone: ZoneId::parse("dev").unwrap(),
            targets: vec![target(ResourceVerb::Get)],
        }
    }

    #[test]
    fn decision_matrix_and_positive_capabilities_are_exact() {
        let context = subject(Locality::Local, EvidenceClass::UnixPeer, "User/alice");
        let engine = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, Some(ResourceVerb::Get), false)),
            test_issuer(),
        )
        .unwrap();
        assert!(engine.authorize(&context, &request(), &state(4)).is_ok());
        let caps = engine
            .positive_capabilities(&context, &ZoneId::parse("dev").unwrap(), &state(4))
            .unwrap();
        assert_eq!(caps.resources, vec![target(ResourceVerb::Get)]);
        assert_eq!(
            engine
                .authorize(
                    &context,
                    &AuthorizationRequest {
                        targets: vec![target(ResourceVerb::Delete)],
                        ..request()
                    },
                    &state(4),
                )
                .unwrap_err(),
            AuthorizationDenial::NoMatchingGrant
        );
    }

    #[test]
    fn revocation_and_policy_outage_fail_closed() {
        let context = subject(Locality::Local, EvidenceClass::UnixPeer, "User/alice");
        let engine = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, Some(ResourceVerb::Get), false)),
            test_issuer(),
        )
        .unwrap();
        assert!(engine.authorize(&context, &request(), &state(4)).is_ok());
        engine
            .replace_policy(policy(5, &context, None, false), &state(5))
            .unwrap();
        assert_eq!(
            engine
                .authorize(&context, &request(), &state(5))
                .unwrap_err(),
            AuthorizationDenial::NoMatchingGrant
        );
        engine.mark_policy_unavailable();
        assert_eq!(
            engine
                .authorize(&context, &request(), &state(5))
                .unwrap_err(),
            AuthorizationDenial::PolicyUnavailable
        );
    }

    #[test]
    fn same_revision_policy_replacement_invalidates_a_cached_allow() {
        let context = subject(Locality::Local, EvidenceClass::UnixPeer, "User/alice");
        let engine = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, Some(ResourceVerb::Get), false)),
            test_issuer(),
        )
        .unwrap();
        assert!(engine.authorize(&context, &request(), &state(4)).is_ok());

        engine
            .replace_policy(policy(4, &context, None, false), &state(4))
            .unwrap();

        assert_eq!(
            engine
                .authorize(&context, &request(), &state(4))
                .unwrap_err(),
            AuthorizationDenial::NoMatchingGrant
        );
    }

    #[test]
    fn replacement_and_permit_minting_are_linearized() {
        use std::sync::mpsc::{self, TryRecvError};

        let context = subject(Locality::Local, EvidenceClass::UnixPeer, "User/alice");
        let engine = Arc::new(
            NativeAuthorizer::from_issuer(
                ApiCatalog::standard(),
                Some(policy(4, &context, Some(ResourceVerb::Get), false)),
                test_issuer(),
            )
            .unwrap(),
        );
        assert!(engine.authorize(&context, &request(), &state(4)).is_ok());

        let (at_grant_tx, at_grant_rx) = mpsc::channel();
        let (release_grant_tx, release_grant_rx) = mpsc::channel();
        let authorizing_engine = Arc::clone(&engine);
        let authorizing_context = context.clone();
        let authorizing = std::thread::spawn(move || {
            authorizing_engine.authorize_before_grant(
                &authorizing_context,
                &request(),
                &state(4),
                || {
                    at_grant_tx.send(()).unwrap();
                    release_grant_rx.recv().unwrap();
                },
            )
        });
        at_grant_rx.recv().unwrap();
        assert!(
            engine.policy.try_write().is_err(),
            "permit minting released the policy guard before returning"
        );

        let (replacement_started_tx, replacement_started_rx) = mpsc::channel();
        let (replacement_done_tx, replacement_done_rx) = mpsc::channel();
        let replacing_engine = Arc::clone(&engine);
        let replacing_context = context.clone();
        let replacing = std::thread::spawn(move || {
            replacement_started_tx.send(()).unwrap();
            let result = replacing_engine
                .replace_policy(policy(4, &replacing_context, None, false), &state(4));
            replacement_done_tx.send(result).unwrap();
        });
        replacement_started_rx.recv().unwrap();
        assert_eq!(replacement_done_rx.try_recv(), Err(TryRecvError::Empty));

        release_grant_tx.send(()).unwrap();
        assert!(authorizing.join().unwrap().is_ok());
        replacement_done_rx.recv().unwrap().unwrap();
        replacing.join().unwrap();

        assert_eq!(
            engine
                .authorize(&context, &request(), &state(4))
                .unwrap_err(),
            AuthorizationDenial::NoMatchingGrant
        );
    }

    #[test]
    fn adjacent_route_cannot_disable_relay_admission() {
        let context = subject(
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
            "ZoneLink/parent",
        );
        let no_relay = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, Some(ResourceVerb::Get), false)),
            test_issuer(),
        )
        .unwrap();
        assert_eq!(
            no_relay
                .authorize(&context, &request(), &state(4))
                .unwrap_err(),
            AuthorizationDenial::RelayGrantMissing
        );
        let no_target = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, None, true)),
            test_issuer(),
        )
        .unwrap();
        assert_eq!(
            no_target
                .authorize(&context, &request(), &state(4))
                .unwrap_err(),
            AuthorizationDenial::RelayTargetGrantMissing
        );
        let both = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, Some(ResourceVerb::Get), true)),
            test_issuer(),
        )
        .unwrap();
        assert!(both.authorize(&context, &request(), &state(4)).is_ok());
    }

    #[test]
    fn relay_rejects_untrusted_adjacent_and_remote_origins() {
        for (locality, evidence) in [
            (Locality::AdjacentZone, EvidenceClass::BootstrapIkpsk2),
            (Locality::Remote, EvidenceClass::EnrolledKk),
        ] {
            let context = subject(locality, evidence, "ZoneLink/parent");
            let engine = NativeAuthorizer::from_issuer(
                ApiCatalog::standard(),
                Some(policy(4, &context, Some(ResourceVerb::Get), true)),
                test_issuer(),
            )
            .unwrap();
            assert_eq!(
                engine
                    .authorize(&context, &request(), &state(4))
                    .unwrap_err(),
                AuthorizationDenial::RelayOriginInvalid
            );
        }
    }

    #[test]
    fn positive_cache_cannot_cross_authentication_evidence() {
        let enrolled = subject(
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
            "ZoneLink/parent",
        );
        let engine = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &enrolled, Some(ResourceVerb::Get), true)),
            test_issuer(),
        )
        .unwrap();
        assert!(engine.authorize(&enrolled, &request(), &state(4)).is_ok());

        let bootstrap = subject(
            Locality::AdjacentZone,
            EvidenceClass::BootstrapIkpsk2,
            "ZoneLink/parent",
        );
        assert_eq!(
            engine
                .authorize(&bootstrap, &request(), &state(4))
                .unwrap_err(),
            AuthorizationDenial::RelayOriginInvalid
        );
    }

    #[test]
    fn parent_subject_cannot_cross_the_child_zone_boundary() {
        let context = subject(
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
            "ZoneLink/parent",
        );
        let engine = NativeAuthorizer::from_issuer(
            ApiCatalog::standard(),
            Some(policy(4, &context, Some(ResourceVerb::Get), true)),
            test_issuer(),
        )
        .unwrap();
        let mut wrong_zone = request();
        wrong_zone.zone = ZoneId::parse("other").unwrap();
        assert_eq!(
            engine
                .authorize(&context, &wrong_zone, &state(4))
                .unwrap_err(),
            AuthorizationDenial::ZoneMismatch
        );
    }

    #[test]
    fn closed_rule_bounds_and_relay_origin_are_validated() {
        let too_many_types = (0..=MAX_ROLE_RULE_RESOURCE_TYPES)
            .map(|index| ResourceTypeName::parse(format!("p{index}.d2bus.org.Type")).unwrap())
            .collect::<Vec<_>>();
        let extension_catalog = ApiCatalog::with_extensions(too_many_types.clone()).unwrap();
        assert_eq!(
            PolicyRule::new(
                &extension_catalog,
                too_many_types,
                [ResourceVerb::Get],
                [],
                [],
                [],
                [],
                [],
            ),
            Err(AuthorizationPolicyError::RuleBounds)
        );

        let context = subject(
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
            "ZoneLink/parent",
        );
        let relay_role = CompiledRole::new(
            ResourceRef::parse("Role/relay").unwrap(),
            vec![
                PolicyRule::new(
                    &ApiCatalog::standard(),
                    [],
                    [],
                    [SessionVerb::Relay],
                    [],
                    [],
                    [],
                    [],
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let binding = CompiledRoleBinding::new(
            relay_role.role_ref.clone(),
            [BoundSubject {
                subject_ref: context.subject_ref().clone(),
                subject_uid: context.subject_uid().clone(),
            }],
            BindingScope::default(),
            RelayGrantAuthority::None,
        )
        .unwrap();
        assert_eq!(
            PolicySet::new(&ApiCatalog::standard(), 4, vec![relay_role], vec![binding]),
            Err(AuthorizationPolicyError::RelayGrantRestricted)
        );

        assert_eq!(
            PolicyRule::new(
                &ApiCatalog::standard(),
                [ResourceTypeName::parse("Credential").unwrap()],
                [ResourceVerb::AdminCredential],
                [],
                ["create".to_owned()],
                [],
                [],
                [],
            ),
            Err(AuthorizationPolicyError::CredentialScope)
        );

        let invalid_subject = BoundSubject {
            subject_ref: ResourceRef::parse("Credential/signing").unwrap(),
            subject_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
        };
        assert_eq!(
            CompiledRoleBinding::new(
                ResourceRef::parse("Role/operator").unwrap(),
                [invalid_subject],
                BindingScope::default(),
                RelayGrantAuthority::None,
            ),
            Err(AuthorizationPolicyError::BindingShape)
        );
    }

    #[test]
    fn uninstalled_resource_types_are_rejected_in_rules_and_targets() {
        let catalog = ApiCatalog::standard();
        let extension = ResourceTypeName::parse("example.d2bus.org.Widget").unwrap();
        assert_eq!(
            PolicyRule::new(
                &catalog,
                [extension.clone()],
                [ResourceVerb::Get],
                [],
                [],
                [],
                [],
                [],
            ),
            Err(AuthorizationPolicyError::UnknownResourceType)
        );

        let context = subject(Locality::Local, EvidenceClass::UnixPeer, "User/alice");
        let engine = NativeAuthorizer::from_issuer(catalog, None, test_issuer()).unwrap();
        let mut uninstalled = request();
        uninstalled.targets[0].resource_type = extension;
        assert_eq!(
            engine
                .authorize(&context, &uninstalled, &state(4))
                .unwrap_err(),
            AuthorizationDenial::UnknownResourceType
        );
    }

    #[test]
    fn configuration_revision_is_a_monotonic_ordinal_in_the_snapshot() {
        let snapshot = state(4).snapshot;
        assert_eq!(snapshot.active_configuration_revision.get(), 3);
        assert_eq!(
            snapshot
                .active_configuration_revision
                .checked_next()
                .unwrap()
                .get(),
            4
        );
        let _: Option<ConfigurationGeneration> = Some(snapshot.active_configuration_revision);
        let _: Option<ResourceGeneration> = None;
    }

    #[test]
    fn bootstrap_matrix_matches_literal_oracle_and_denies_every_dimension_near_miss() {
        const EXPECTED_BOOTSTRAP_ROWS: [(&str, ApiMethod, &str, ResourceVerb); 42] = [
            (
                "system-core",
                ApiMethod::Create,
                "Zone",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::Create,
                "Provider",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::Create,
                "Host",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::Create,
                "User",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::Create,
                "Role",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::Create,
                "RoleBinding",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::Create,
                "Process",
                ResourceVerb::Create,
            ),
            (
                "system-core",
                ApiMethod::UpdateStatus,
                "Zone",
                ResourceVerb::UpdateStatus,
            ),
            (
                "system-core",
                ApiMethod::UpdateStatus,
                "Provider",
                ResourceVerb::UpdateStatus,
            ),
            (
                "system-core",
                ApiMethod::UpdateStatus,
                "Host",
                ResourceVerb::UpdateStatus,
            ),
            (
                "system-core",
                ApiMethod::UpdateStatus,
                "User",
                ResourceVerb::UpdateStatus,
            ),
            (
                "system-core",
                ApiMethod::UpdateStatus,
                "Process",
                ResourceVerb::UpdateStatus,
            ),
            ("system-core", ApiMethod::Get, "Zone", ResourceVerb::Get),
            ("system-core", ApiMethod::Get, "Provider", ResourceVerb::Get),
            ("system-core", ApiMethod::Get, "Host", ResourceVerb::Get),
            ("system-core", ApiMethod::Get, "User", ResourceVerb::Get),
            ("system-core", ApiMethod::Get, "Process", ResourceVerb::Get),
            ("system-core", ApiMethod::List, "Zone", ResourceVerb::List),
            (
                "system-core",
                ApiMethod::List,
                "Provider",
                ResourceVerb::List,
            ),
            ("system-core", ApiMethod::List, "Host", ResourceVerb::List),
            ("system-core", ApiMethod::List, "User", ResourceVerb::List),
            (
                "system-core",
                ApiMethod::List,
                "Process",
                ResourceVerb::List,
            ),
            ("system-core", ApiMethod::Watch, "Zone", ResourceVerb::Watch),
            (
                "system-core",
                ApiMethod::Watch,
                "Provider",
                ResourceVerb::Watch,
            ),
            ("system-core", ApiMethod::Watch, "Host", ResourceVerb::Watch),
            ("system-core", ApiMethod::Watch, "User", ResourceVerb::Watch),
            (
                "system-core",
                ApiMethod::Watch,
                "Process",
                ResourceVerb::Watch,
            ),
            (
                "system-core",
                ApiMethod::ResolveRef,
                "Zone",
                ResourceVerb::Get,
            ),
            (
                "system-core",
                ApiMethod::ResolveRef,
                "Provider",
                ResourceVerb::Get,
            ),
            (
                "system-core",
                ApiMethod::ResolveRef,
                "Host",
                ResourceVerb::Get,
            ),
            (
                "system-core",
                ApiMethod::ResolveRef,
                "User",
                ResourceVerb::Get,
            ),
            (
                "system-core",
                ApiMethod::ResolveRef,
                "Process",
                ResourceVerb::Get,
            ),
            (
                "system-core",
                ApiMethod::InspectSchema,
                "Provider",
                ResourceVerb::Get,
            ),
            (
                "system-minijail",
                ApiMethod::Get,
                "Process",
                ResourceVerb::Get,
            ),
            (
                "system-minijail",
                ApiMethod::Get,
                "EphemeralProcess",
                ResourceVerb::Get,
            ),
            (
                "system-minijail",
                ApiMethod::List,
                "Process",
                ResourceVerb::List,
            ),
            (
                "system-minijail",
                ApiMethod::List,
                "EphemeralProcess",
                ResourceVerb::List,
            ),
            (
                "system-minijail",
                ApiMethod::Watch,
                "Process",
                ResourceVerb::Watch,
            ),
            (
                "system-minijail",
                ApiMethod::Watch,
                "EphemeralProcess",
                ResourceVerb::Watch,
            ),
            (
                "system-minijail",
                ApiMethod::UpdateStatus,
                "Process",
                ResourceVerb::UpdateStatus,
            ),
            (
                "system-minijail",
                ApiMethod::UpdateStatus,
                "EphemeralProcess",
                ResourceVerb::UpdateStatus,
            ),
            (
                "system-minijail",
                ApiMethod::InspectSchema,
                "Process",
                ResourceVerb::Get,
            ),
        ];

        let actual = BOOTSTRAP_ROWS
            .iter()
            .map(|row| (row.subject_name, row.method, row.resource_type, row.verb))
            .collect::<Vec<_>>();
        assert_eq!(actual.as_slice(), &EXPECTED_BOOTSTRAP_ROWS);

        let core_uid = "123e4567-e89b-42d3-a456-426614174000";
        let minijail_uid = "123e4567-e89b-42d3-a456-426614174001";
        let state = bootstrap_state(BootstrapPhase::Provisioned {
            zone: ZoneId::parse("dev").unwrap(),
            system_core_uid: ResourceUid::parse(core_uid).unwrap(),
            system_minijail_uid: ResourceUid::parse(minijail_uid).unwrap(),
            controller_generation: ControllerGeneration::new(11).unwrap(),
            provider_generation: ResourceGeneration::new(12).unwrap(),
        });
        let engine =
            NativeAuthorizer::from_issuer(ApiCatalog::standard(), None, test_issuer()).unwrap();

        for (subject_name, method, resource_type, verb) in EXPECTED_BOOTSTRAP_ROWS {
            let uid = if subject_name == "system-core" {
                core_uid
            } else {
                minijail_uid
            };
            let context = bootstrap_subject(subject_name, uid);
            let exact = AuthorizationRequest {
                method,
                zone: ZoneId::parse("dev").unwrap(),
                targets: vec![bootstrap_target(resource_type, verb)],
            };
            assert_eq!(
                engine.authorize(&context, &exact, &state).map(|_| ()),
                Ok(()),
                "bootstrap row did not authorize: {} {:?} {} {:?}",
                subject_name,
                method,
                resource_type,
                verb,
            );

            let wrong_subject =
                bootstrap_subject(subject_name, "123e4567-e89b-42d3-a456-426614174099");
            assert_eq!(
                engine
                    .authorize(&wrong_subject, &exact, &state)
                    .unwrap_err(),
                AuthorizationDenial::BootstrapDenied,
                "bootstrap subject near miss authorized: {subject_name} {method:?} {resource_type}"
            );

            let wrong_subject_name = bootstrap_subject("system-subject-near-miss", core_uid);
            let name_mismatch_state = bootstrap_state(BootstrapPhase::Unprovisioned {
                zone: ZoneId::parse("dev").unwrap(),
                controller_generation: ControllerGeneration::new(11).unwrap(),
                provider_generation: ResourceGeneration::new(12).unwrap(),
            });
            assert_eq!(
                engine
                    .authorize(&wrong_subject_name, &exact, &name_mismatch_state)
                    .unwrap_err(),
                AuthorizationDenial::BootstrapDenied,
                "bootstrap subject-name near miss authorized: \
                 {subject_name} {method:?} {resource_type}"
            );

            let mut wrong_verb = exact.clone();
            wrong_verb.targets[0].verb = ResourceVerb::UseCredential;
            assert_eq!(
                engine.authorize(&context, &wrong_verb, &state).unwrap_err(),
                AuthorizationDenial::BootstrapDenied,
                "bootstrap verb near miss authorized: {subject_name} {method:?} {resource_type}"
            );

            let mut wrong_method = exact.clone();
            wrong_method.method = ApiMethod::Delete;
            assert_eq!(
                engine
                    .authorize(&context, &wrong_method, &state)
                    .unwrap_err(),
                AuthorizationDenial::BootstrapDenied,
                "bootstrap method near miss authorized: {subject_name} {method:?} {resource_type}"
            );

            let mut wrong_resource_type = exact.clone();
            wrong_resource_type.targets[0].resource_type =
                ResourceTypeName::parse("Credential").unwrap();
            assert_eq!(
                engine
                    .authorize(&context, &wrong_resource_type, &state)
                    .unwrap_err(),
                AuthorizationDenial::BootstrapDenied,
                "bootstrap resource type near miss authorized: \
                 {subject_name} {method:?} {resource_type}"
            );

            let mut wrong_zone = exact;
            wrong_zone.zone = ZoneId::parse("personal").unwrap();
            assert_eq!(
                engine.authorize(&context, &wrong_zone, &state).unwrap_err(),
                AuthorizationDenial::ZoneMismatch,
                "bootstrap Zone near miss authorized: {subject_name} {method:?} {resource_type}"
            );
        }
    }

    #[test]
    fn bootstrap_zone_and_provider_names_are_compiled_in_both_phases() {
        let core_uid = "123e4567-e89b-42d3-a456-426614174000";
        let context = bootstrap_subject("system-core", core_uid);
        let engine =
            NativeAuthorizer::from_issuer(ApiCatalog::standard(), None, test_issuer()).unwrap();
        let phases = [
            BootstrapPhase::Unprovisioned {
                zone: ZoneId::parse("dev").unwrap(),
                controller_generation: ControllerGeneration::new(11).unwrap(),
                provider_generation: ResourceGeneration::new(12).unwrap(),
            },
            BootstrapPhase::Provisioned {
                zone: ZoneId::parse("dev").unwrap(),
                system_core_uid: ResourceUid::parse(core_uid).unwrap(),
                system_minijail_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174001")
                    .unwrap(),
                controller_generation: ControllerGeneration::new(11).unwrap(),
                provider_generation: ResourceGeneration::new(12).unwrap(),
            },
        ];

        for phase in phases {
            let state = bootstrap_state(phase);
            for resource_type in ["Zone", "Provider"] {
                let mut target = bootstrap_target(resource_type, ResourceVerb::Create);
                target.resource_name = Some(ResourceName::parse("attacker-selected").unwrap());
                let request = AuthorizationRequest {
                    method: ApiMethod::Create,
                    zone: ZoneId::parse("dev").unwrap(),
                    targets: vec![target],
                };
                assert_eq!(
                    engine.authorize(&context, &request, &state).unwrap_err(),
                    AuthorizationDenial::BootstrapDenied
                );
            }
        }
    }

    #[test]
    fn authorization_debug_surfaces_redact_policy_and_identity_fields() {
        const ZONE_SENTINEL: &str = "authz-zone-sentinel";
        const NAME_SENTINEL: &str = "authz-name-sentinel";
        const REF_SENTINEL: &str = "authz-ref-sentinel";
        const UID_SENTINEL: &str = "33333333-3333-4333-8333-333333333333";
        const PAYLOAD_SENTINEL: &str = "authz-payload-sentinel";
        const TYPE_SENTINEL: &str = "authz-sentinel.d2bus.org.Widget";

        let extension = ResourceTypeName::parse(TYPE_SENTINEL).unwrap();
        let catalog = ApiCatalog::with_extensions([extension.clone()]).unwrap();
        let target = AuthorizationTarget {
            resource_type: extension.clone(),
            resource_name: Some(ResourceName::parse(NAME_SENTINEL).unwrap()),
            verb: ResourceVerb::Get,
            subresource: Some(PAYLOAD_SENTINEL.to_owned()),
            execution_ref: Some(ResourceRef::parse(&format!("Process/{REF_SENTINEL}")).unwrap()),
        };
        let request = AuthorizationRequest {
            method: ApiMethod::Get,
            zone: ZoneId::parse(ZONE_SENTINEL).unwrap(),
            targets: vec![target.clone()],
        };
        let mut protected_state = state(9);
        protected_state.bootstrap_phase = BootstrapPhase::Unprovisioned {
            zone: ZoneId::parse(ZONE_SENTINEL).unwrap(),
            controller_generation: ControllerGeneration::new(11).unwrap(),
            provider_generation: ResourceGeneration::new(12).unwrap(),
        };
        let bound_subject = BoundSubject {
            subject_ref: ResourceRef::parse(&format!("User/{REF_SENTINEL}")).unwrap(),
            subject_uid: ResourceUid::parse(UID_SENTINEL).unwrap(),
        };
        let scope = BindingScope {
            zones: BTreeSet::from([ZoneId::parse(ZONE_SENTINEL).unwrap()]),
            resource_names: BTreeSet::from([ResourceName::parse(NAME_SENTINEL).unwrap()]),
            execution_refs: BTreeSet::from([ResourceRef::parse(&format!(
                "Process/{REF_SENTINEL}"
            ))
            .unwrap()]),
        };
        let rule = PolicyRule::new(
            &catalog,
            [extension],
            [ResourceVerb::Get],
            [SessionVerb::Connect],
            [PAYLOAD_SENTINEL.to_owned()],
            [ResourceName::parse(NAME_SENTINEL).unwrap()],
            [ZoneId::parse(ZONE_SENTINEL).unwrap()],
            [ResourceRef::parse(&format!("Process/{REF_SENTINEL}")).unwrap()],
        )
        .unwrap();
        let role = CompiledRole::new(
            ResourceRef::parse(&format!("Role/{REF_SENTINEL}")).unwrap(),
            vec![rule.clone()],
        )
        .unwrap();
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [bound_subject.clone()],
            scope.clone(),
            RelayGrantAuthority::None,
        )
        .unwrap();
        let policy =
            PolicySet::new(&catalog, 9, vec![role.clone()], vec![binding.clone()]).unwrap();
        let capabilities = PositiveCapabilities {
            resources: vec![target.clone()],
            session_verbs: BTreeSet::from([SessionVerb::Connect]),
        };
        let context = subject(
            Locality::Local,
            EvidenceClass::UnixPeer,
            &format!("User/{REF_SENTINEL}"),
        );
        let grant = grant(&test_issuer(), &context, &request, protected_state.snapshot);
        let authorizer =
            NativeAuthorizer::from_issuer(catalog.clone(), Some(policy.clone()), test_issuer())
                .unwrap();

        for rendered in [
            format!("{catalog:?}"),
            format!("{target:?}"),
            format!("{request:?}"),
            format!("{protected_state:?}"),
            format!("{:?}", protected_state.bootstrap_phase),
            format!("{bound_subject:?}"),
            format!("{scope:?}"),
            format!("{rule:?}"),
            format!("{role:?}"),
            format!("{binding:?}"),
            format!("{policy:?}"),
            format!("{capabilities:?}"),
            format!("{grant:?}"),
            format!("{authorizer:?}"),
        ] {
            for sentinel in [
                ZONE_SENTINEL,
                NAME_SENTINEL,
                REF_SENTINEL,
                UID_SENTINEL,
                PAYLOAD_SENTINEL,
                TYPE_SENTINEL,
            ] {
                assert!(!rendered.contains(sentinel), "{rendered}");
            }
        }
    }
}
