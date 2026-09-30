//! The `ExecutionPolicy` ResourceType: reusable confinement, nothing else.
//!
//! An `ExecutionPolicy` row states what an execution instance is allowed to
//! be - the isolation classes it must run under, the Linux capabilities it may
//! hold, the restrictions it may not weaken, the identity it may resolve to,
//! and the syscall filter it must load. It grants no storage, device,
//! network, endpoint, or credential access: those are separate typed binding
//! relationships, and a policy that named them would be a second authority.
//!
//! # Composition is field-wise, and conflict is a refusal
//!
//! [`admit_execution`] evaluates an instance request and its provider's
//! declared requirements against one authorized policy and one target's
//! declared support. Composition is not set intersection: a missing required
//! class, a capability outside the ceiling, a weakened mandatory
//! restriction, an unauthorized identity, or a mandatory facet the target
//! cannot enforce is refused, never silently dropped, relaxed, or replaced
//! with a broader alternative.
//!
//! Selecting this resource is a request, not an authorization. The caller
//! must present authorization evidence produced by the Role and RoleBinding
//! contracts; a syntactically valid reference grants nothing on its own.
//!
//! # The old flattened fragment is not this resource
//!
//! The `Host` and `Guest` base specs still flatten an `ExecutionPolicy`
//! fragment that carries a default domain, a fallback user, a budget, and
//! attachment defaults. That value is execution-parent facts plus an
//! overloaded attachment list, and it is not this ResourceType. It is
//! converted field by field, and the attachment half becomes either a
//! graph-backed target-support constraint, a binding request for the parent
//! itself, or a default that can only shape a child request.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    authority::{AdmissionStage, RefusalReason},
    execution_policy::{PrimitiveSpecError, ensure_unique, redacted_debug, require_resource_type},
    process::{CapabilityClass, NamespaceClass},
    seccomp_profile::SECCOMP_PROFILE_RESOURCE_TYPE,
};
use d2b_contracts::wire_deserialize;

/// Canonical `ExecutionPolicy` ResourceType name.
pub const EXECUTION_POLICY_RESOURCE_TYPE: &str = "ExecutionPolicy";
/// The only Provider admitted by `ExecutionPolicy.spec.providerRef`.
pub const EXECUTION_POLICY_PROVIDER_REF: &str = "Provider/execution-policy";
/// Maximum namespace classes one policy requires.
pub const MAX_POLICY_NAMESPACE_CLASSES: usize = 8;
/// Maximum capability classes one policy admits.
pub const MAX_POLICY_CAPABILITY_CLASSES: usize = 16;
/// Highest umask one policy may request.
pub const MAX_POLICY_UMASK: u32 = 0o777;
/// Maximum requested or admitted millicpus.
pub const MAX_POLICY_MILLICPU: u64 = 1_024_000;
/// Maximum requested or admitted memory bytes.
pub const MAX_POLICY_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024 * 1024;
/// Maximum requested or admitted process ids.
pub const MAX_POLICY_PIDS: u64 = 65_535;
/// Maximum requested or admitted file descriptors.
pub const MAX_POLICY_FDS: u64 = 1_048_576;

/// One refusal, carrying the stage that enforced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyRefusal {
    /// The stage that refused.
    stage: AdmissionStage,
    /// The typed reason it refused.
    reason: RefusalReason,
}

impl PolicyRefusal {
    /// Construct a refusal.
    pub const fn new(stage: AdmissionStage, reason: RefusalReason) -> Self {
        Self { stage, reason }
    }

    /// Borrow the enforcing stage.
    pub const fn stage(&self) -> AdmissionStage {
        self.stage
    }

    /// Borrow the refusal reason.
    pub const fn reason(&self) -> RefusalReason {
        self.reason
    }
}

impl core::fmt::Display for PolicyRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?} refused at {:?}", self.reason, self.stage)
    }
}

impl std::error::Error for PolicyRefusal {}

/// The isolation classes one policy requires.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyNamespaces {
    classes: Vec<NamespaceClass>,
}

impl PolicyNamespaces {
    /// Construct the required set after checking its bound and uniqueness.
    pub fn new(classes: Vec<NamespaceClass>) -> Result<Self, PrimitiveSpecError> {
        if classes.len() > MAX_POLICY_NAMESPACE_CLASSES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        ensure_unique(&classes)?;
        Ok(Self { classes })
    }

    /// Borrow the required classes.
    pub fn classes(&self) -> &[NamespaceClass] {
        &self.classes
    }

    /// Whether this policy requires `class`.
    pub fn requires(&self, class: NamespaceClass) -> bool {
        self.classes.contains(&class)
    }
}

redacted_debug!(PolicyNamespaces);

/// The Linux capabilities one policy admits.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyCapabilities {
    allowed: Vec<CapabilityClass>,
}

impl PolicyCapabilities {
    /// Construct the admitted ceiling after checking its bound and uniqueness.
    pub fn new(allowed: Vec<CapabilityClass>) -> Result<Self, PrimitiveSpecError> {
        if allowed.len() > MAX_POLICY_CAPABILITY_CLASSES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        ensure_unique(&allowed)?;
        Ok(Self { allowed })
    }

    /// Borrow the admitted classes.
    pub fn allowed(&self) -> &[CapabilityClass] {
        &self.allowed
    }

    /// Whether this policy admits `class`.
    pub fn admits(&self, class: CapabilityClass) -> bool {
        self.allowed.contains(&class)
    }
}

redacted_debug!(PolicyCapabilities);

/// The identity rules one policy authorizes.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyIdentity {
    user_ref: Option<ResourceRef>,
    require_user_namespace: bool,
}

impl PolicyIdentity {
    /// Construct the identity rules.
    pub fn new(user_ref: Option<ResourceRef>, require_user_namespace: bool) -> Result<Self, PrimitiveSpecError> {
        if let Some(user_ref) = &user_ref {
            require_resource_type(user_ref, "User")?;
        }
        if require_user_namespace && user_ref.is_none() {
            return Err(PrimitiveSpecError::MissingRequiredField);
        }
        Ok(Self {
            user_ref,
            require_user_namespace,
        })
    }

    /// The exact identity this policy authorizes, when it names one.
    pub const fn user_ref(&self) -> Option<&ResourceRef> {
        self.user_ref.as_ref()
    }

    /// Whether the instance must run under a user namespace.
    pub const fn requires_user_namespace(&self) -> bool {
        self.require_user_namespace
    }
}

redacted_debug!(PolicyIdentity);

/// The root-filesystem restrictions one policy mandates.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyRoot {
    read_only_root: bool,
    private_root: bool,
}

impl PolicyRoot {
    /// Construct the root restrictions.
    pub const fn new(read_only_root: bool, private_root: bool) -> Self {
        Self {
            read_only_root,
            private_root,
        }
    }

    /// Whether the root filesystem is read-only.
    pub const fn read_only_root(&self) -> bool {
        self.read_only_root
    }

    /// Whether the instance receives a private root.
    pub const fn private_root(&self) -> bool {
        self.private_root
    }
}

redacted_debug!(PolicyRoot);

/// The syscall filter one policy selects.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicySeccomp {
    profile_ref: Option<ResourceRef>,
}

impl PolicySeccomp {
    /// Construct the syscall-filter selection.
    pub fn new(profile_ref: Option<ResourceRef>) -> Result<Self, PrimitiveSpecError> {
        if let Some(profile_ref) = &profile_ref {
            require_resource_type(profile_ref, SECCOMP_PROFILE_RESOURCE_TYPE)?;
        }
        Ok(Self { profile_ref })
    }

    /// The selected syscall-filter profile.
    pub const fn profile_ref(&self) -> Option<&ResourceRef> {
        self.profile_ref.as_ref()
    }
}

redacted_debug!(PolicySeccomp);

/// The `ExecutionPolicy` desired spec.
///
/// There is deliberately no volume, device, network, endpoint, credential,
/// mount, or host-path field: resource access is admitted through the typed
/// binding relationships, and a field here would reintroduce the
/// independent attachment grant this resource replaces.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPolicySpec {
    namespaces: PolicyNamespaces,
    capabilities: PolicyCapabilities,
    no_new_privileges: bool,
    identity: PolicyIdentity,
    root: PolicyRoot,
    seccomp: PolicySeccomp,
    umask: Option<u32>,
}

impl ExecutionPolicySpec {
    /// Construct a policy after checking every frozen bound.
    pub fn new(
        namespaces: PolicyNamespaces,
        capabilities: PolicyCapabilities,
        no_new_privileges: bool,
        identity: PolicyIdentity,
        root: PolicyRoot,
        seccomp: PolicySeccomp,
        umask: Option<u32>,
    ) -> Result<Self, PrimitiveSpecError> {
        if umask.is_some_and(|value| value > MAX_POLICY_UMASK) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            namespaces,
            capabilities,
            no_new_privileges,
            identity,
            root,
            seccomp,
            umask,
        })
    }

    /// Borrow the required isolation classes.
    pub const fn namespaces(&self) -> &PolicyNamespaces {
        &self.namespaces
    }

    /// Borrow the admitted capability ceiling.
    pub const fn capabilities(&self) -> &PolicyCapabilities {
        &self.capabilities
    }

    /// Whether privilege escalation is disabled.
    pub const fn no_new_privileges(&self) -> bool {
        self.no_new_privileges
    }

    /// Borrow the authorized identity rules.
    pub const fn identity(&self) -> &PolicyIdentity {
        &self.identity
    }

    /// Borrow the mandated root restrictions.
    pub const fn root(&self) -> &PolicyRoot {
        &self.root
    }

    /// Borrow the selected syscall filter.
    pub const fn seccomp(&self) -> &PolicySeccomp {
        &self.seccomp
    }

    /// The requested umask, when the policy pins one.
    pub const fn umask(&self) -> Option<u32> {
        self.umask
    }

    /// The mandatory confinement facets this policy implies.
    ///
    /// These are the facets an implementation must be able to enforce; a
    /// target missing one of them cannot admit an instance under this policy
    /// at all, which is a refusal rather than a silently broader launch.
    pub fn mandatory_facets(&self) -> Vec<ConfinementFacet> {
        let mut facets = Vec::new();
        for class in self.namespaces.classes() {
            facets.push(ConfinementFacet::from_namespace(*class));
        }
        if self.no_new_privileges {
            facets.push(ConfinementFacet::NoNewPrivileges);
        }
        if self.root.read_only_root {
            facets.push(ConfinementFacet::ReadOnlyRoot);
        }
        if self.root.private_root {
            facets.push(ConfinementFacet::PrivateRoot);
        }
        if !self.capabilities.allowed().is_empty() {
            facets.push(ConfinementFacet::CapabilityCeiling);
        }
        if self.seccomp.profile_ref().is_some() {
            facets.push(ConfinementFacet::SyscallFilter);
        }
        facets
    }
}

redacted_debug!(ExecutionPolicySpec);

wire_deserialize!(
    ExecutionPolicySpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        #[serde(default)]
        namespaces: PolicyNamespaces,
        #[serde(default)]
        capabilities: PolicyCapabilities,
        #[serde(default)]
        no_new_privileges: bool,
        #[serde(default)]
        identity: PolicyIdentity,
        #[serde(default)]
        root: PolicyRoot,
        #[serde(default)]
        seccomp: PolicySeccomp,
        #[serde(default)]
        umask: Option<u32>,
    },
    wire,
    ExecutionPolicySpec::new(
        wire.namespaces,
        wire.capabilities,
        wire.no_new_privileges,
        wire.identity,
        wire.root,
        wire.seccomp,
        wire.umask,
    )
    .map_err(serde::de::Error::custom)
);

/// The digest of the exact policy spec one instance was admitted under.
///
/// An effect is fenced against the policy that admitted it, not against the
/// spec generation its status happens to report: editing a policy row's
/// restrictions changes this digest even when the rest of the graph is
/// untouched, and the earlier admission no longer matches.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionPolicyFingerprint(String);

impl ExecutionPolicyFingerprint {
    /// The domain tag framing one policy-spec digest.
    pub const DOMAIN_TAG: &'static str = "d2b:v3:execution-policy";

    /// Frame the digest of one committed policy spec.
    pub fn from_spec(spec: &ExecutionPolicySpec) -> Self {
        let bytes = crate::v3::resource_schema::canonical_json_bytes(spec)
            .expect("a policy spec always renders as canonical bytes");
        Self(crate::v3::resource_schema::framed_canonical_digest(
            Self::DOMAIN_TAG,
            &bytes,
        ))
    }

    /// Parse a framed policy-spec digest.
    ///
    /// # Errors
    ///
    /// Returns `OutOfRange` unless the value is a framed canonical digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, PolicyContractError> {
        let value = value.into();
        if crate::v3::resource_schema::is_canonical_digest(&value) {
            Ok(Self(value))
        } else {
            Err(PolicyContractError::OutOfRange)
        }
    }

    /// Borrow the framed digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(ExecutionPolicyFingerprint);

impl schemars::JsonSchema for ExecutionPolicyFingerprint {
    fn schema_name() -> String {
        "ExecutionPolicyFingerprint".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        crate::v3::execution_policy::string_schema_object(1, 128)
    }
}

/// One mandatory confinement facet an implementation must enforce.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ConfinementFacet {
    /// A private user namespace with a resolved identity map.
    UserNamespace,
    /// A private mount namespace with disabled propagation.
    MountNamespace,
    /// A private network namespace.
    NetworkNamespace,
    /// A private PID namespace.
    PidNamespace,
    /// A private IPC namespace.
    IpcNamespace,
    /// A private UTS namespace.
    UtsNamespace,
    /// A private cgroup namespace.
    CgroupNamespace,
    /// A private time namespace.
    TimeNamespace,
    /// A root filesystem restricted to read-only.
    ReadOnlyRoot,
    /// A private root that sibling host content cannot reach.
    PrivateRoot,
    /// Privilege escalation disabled.
    NoNewPrivileges,
    /// An effective Linux capability set bounded by the policy ceiling.
    CapabilityCeiling,
    /// A compiled syscall filter loaded before the workload starts.
    SyscallFilter,
}

impl ConfinementFacet {
    /// The facet a required namespace class implies.
    pub const fn from_namespace(class: NamespaceClass) -> Self {
        match class {
            NamespaceClass::User => Self::UserNamespace,
            NamespaceClass::Mount => Self::MountNamespace,
            NamespaceClass::Network => Self::NetworkNamespace,
            NamespaceClass::Pid => Self::PidNamespace,
            NamespaceClass::Ipc => Self::IpcNamespace,
            NamespaceClass::Uts => Self::UtsNamespace,
            NamespaceClass::Cgroup => Self::CgroupNamespace,
            NamespaceClass::Time => Self::TimeNamespace,
        }
    }
}

/// Every confinement facet, in the closed contract's own order.
///
/// A backend support set is a subset of this. The constant exists so a
/// caller can state "this target enforces everything" without restating the
/// vocabulary, which would otherwise be a second place to forget a variant.
pub const ALL_CONFINEMENT_FACETS: [ConfinementFacet; 13] = [
    ConfinementFacet::UserNamespace,
    ConfinementFacet::MountNamespace,
    ConfinementFacet::NetworkNamespace,
    ConfinementFacet::PidNamespace,
    ConfinementFacet::IpcNamespace,
    ConfinementFacet::UtsNamespace,
    ConfinementFacet::CgroupNamespace,
    ConfinementFacet::TimeNamespace,
    ConfinementFacet::ReadOnlyRoot,
    ConfinementFacet::PrivateRoot,
    ConfinementFacet::NoNewPrivileges,
    ConfinementFacet::CapabilityCeiling,
    ConfinementFacet::SyscallFilter,
];

/// The class of execution instance being admitted.
///
/// The kind exists so an owner can state it explicitly, and deliberately
/// changes nothing: a run-to-completion `EphemeralProcess` and a
/// long-running `Process` pass through exactly the same policy path.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionInstanceKind {
    /// A long-running `Process`.
    LongRunning,
    /// A run-to-completion `EphemeralProcess`.
    OneShot,
}

/// The limits one instance asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetRequest {
    millicpu: u64,
    memory_bytes: u64,
    pids: u64,
    fds: u64,
}

impl BudgetRequest {
    /// Construct a request after checking its frozen bound.
    pub const fn new(millicpu: u64, memory_bytes: u64, pids: u64, fds: u64) -> Result<Self, PolicyContractError> {
        if millicpu > MAX_POLICY_MILLICPU
            || memory_bytes > MAX_POLICY_MEMORY_BYTES
            || pids > MAX_POLICY_PIDS
            || fds > MAX_POLICY_FDS
        {
            return Err(PolicyContractError::OutOfRange);
        }
        Ok(Self {
            millicpu,
            memory_bytes,
            pids,
            fds,
        })
    }

    /// The requested millicpus.
    pub const fn millicpu(&self) -> u64 {
        self.millicpu
    }

    /// The requested memory bytes.
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    /// The requested process ids.
    pub const fn pids(&self) -> u64 {
        self.pids
    }

    /// The requested file descriptors.
    pub const fn fds(&self) -> u64 {
        self.fds
    }
}

/// The admitted limits for one instance.
///
/// The budget and Quota contracts own these numbers. This row is the result
/// of that evaluation, carried so the execution plan cannot restate a wider
/// limit than the one that was admitted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BudgetCeiling {
    millicpu: u64,
    memory_bytes: u64,
    pids: u64,
    fds: u64,
}

impl BudgetCeiling {
    /// Construct a ceiling.
    pub const fn new(millicpu: u64, memory_bytes: u64, pids: u64, fds: u64) -> Self {
        Self {
            millicpu,
            memory_bytes,
            pids,
            fds,
        }
    }

    /// Whether `request` fits entirely inside this ceiling.
    const fn admits(&self, request: &BudgetRequest) -> bool {
        request.millicpu <= self.millicpu
            && request.memory_bytes <= self.memory_bytes
            && request.pids <= self.pids
            && request.fds <= self.fds
    }
}

/// One execution instance's request.
///
/// The selected policy row is resolved by the graph, which owns the typed
/// `ExecutionPolicy` reference and the authorization evidence for it. This
/// value carries the instance's own facts only, so admission can be
/// evaluated - and refused - without a reference in hand.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionInstance {
    kind: ExecutionInstanceKind,
    user_ref: Option<ResourceRef>,
    budget: BudgetRequest,
}

impl ExecutionInstance {
    /// Construct an instance request.
    ///
    /// # Errors
    ///
    /// Returns `WrongUserResourceType` unless the named identity is a
    /// `User`. There is no numerical host credential here to override, and
    /// no host path to point somewhere else.
    pub fn new(
        kind: ExecutionInstanceKind,
        user_ref: Option<ResourceRef>,
        budget: BudgetRequest,
    ) -> Result<Self, PolicyContractError> {
        if let Some(user_ref) = &user_ref {
            require_resource_type(user_ref, "User")
                .map_err(|_| PolicyContractError::WrongUserResourceType)?;
        }
        Ok(Self {
            kind,
            user_ref,
            budget,
        })
    }

    /// Whether the instance is long-running or run-to-completion.
    pub const fn kind(&self) -> ExecutionInstanceKind {
        self.kind
    }

    /// Borrow the requested identity, when the instance names one.
    pub const fn user_ref(&self) -> Option<&ResourceRef> {
        self.user_ref.as_ref()
    }

    /// Borrow the requested limits.
    pub const fn budget(&self) -> &BudgetRequest {
        &self.budget
    }
}

redacted_debug!(ExecutionInstance);

/// What an instance and its selected provider declare they need.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionRequirements {
    namespace_classes: Vec<NamespaceClass>,
    capability_classes: Vec<CapabilityClass>,
    mandatory_no_new_privileges: bool,
    seccomp_profile_ref: Option<ResourceRef>,
    mandatory_facets: Vec<ConfinementFacet>,
}

impl ExecutionRequirements {
    /// Construct a requirement set after checking its bounds and references.
    pub fn new(
        namespace_classes: Vec<NamespaceClass>,
        capability_classes: Vec<CapabilityClass>,
        mandatory_no_new_privileges: bool,
        seccomp_profile_ref: Option<ResourceRef>,
        mandatory_facets: Vec<ConfinementFacet>,
    ) -> Result<Self, PolicyContractError> {
        if namespace_classes.len() > MAX_POLICY_NAMESPACE_CLASSES
            || capability_classes.len() > MAX_POLICY_CAPABILITY_CLASSES
        {
            return Err(PolicyContractError::TooManyEntries);
        }
        if let Some(profile_ref) = &seccomp_profile_ref
            && require_resource_type(profile_ref, SECCOMP_PROFILE_RESOURCE_TYPE).is_err()
        {
            return Err(PolicyContractError::WrongSeccompResourceType);
        }
        Ok(Self {
            namespace_classes,
            capability_classes,
            mandatory_no_new_privileges,
            seccomp_profile_ref,
            mandatory_facets,
        })
    }

    /// The isolation classes the implementation requires.
    pub fn namespace_classes(&self) -> &[NamespaceClass] {
        &self.namespace_classes
    }

    /// The capabilities the implementation requires.
    pub fn capability_classes(&self) -> &[CapabilityClass] {
        &self.capability_classes
    }

    /// Whether the implementation cannot run without privilege escalation
    /// disabled.
    pub const fn mandatory_no_new_privileges(&self) -> bool {
        self.mandatory_no_new_privileges
    }

    /// The syscall filter the implementation requires.
    pub const fn seccomp_profile_ref(&self) -> Option<&ResourceRef> {
        self.seccomp_profile_ref.as_ref()
    }

    /// The additional facets the implementation declares mandatory.
    pub fn mandatory_facets(&self) -> &[ConfinementFacet] {
        &self.mandatory_facets
    }
}

/// The confinement facets one execution backend declares it can enforce.
///
/// This is a closed implementation-contract facet, not a family switch: a
/// backend that cannot enforce a mandatory facet is refused at admission,
/// before any host effect, rather than launching without it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackendSupport {
    facets: Vec<ConfinementFacet>,
}

impl BackendSupport {
    /// Construct a support set, requiring unique entries.
    pub fn new(facets: Vec<ConfinementFacet>) -> Result<Self, PolicyContractError> {
        let mut sorted = facets.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != facets.len() {
            return Err(PolicyContractError::TooManyEntries);
        }
        Ok(Self { facets })
    }

    /// Whether the backend enforces `facet`.
    pub fn enforces(&self, facet: ConfinementFacet) -> bool {
        self.facets.contains(&facet)
    }
}

/// Authorization evidence for one policy selection.
///
/// The Role and RoleBinding contracts produce this. A reference to a policy
/// row is a request to use it, never a grant: without this evidence the
/// selection is refused, and a candidate that would introduce its own grant
/// is evaluated against prior accepted authority, not against itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyAuthorization {
    granted: bool,
}

impl PolicyAuthorization {
    /// Authorization evidence for a permitted selection.
    pub const fn granted() -> Self {
        Self { granted: true }
    }

    /// The absence of authorization evidence.
    pub const fn absent() -> Self {
        Self { granted: false }
    }

    /// Whether the selection is authorized.
    pub const fn is_granted(&self) -> bool {
        self.granted
    }
}

/// One admitted execution: the effective, non-widening result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdmittedExecution {
    kind: ExecutionInstanceKind,
    policy: ExecutionPolicyFingerprint,
    namespace_classes: Vec<NamespaceClass>,
    capability_classes: Vec<CapabilityClass>,
    no_new_privileges: bool,
    user_ref: Option<ResourceRef>,
    seccomp_profile_ref: Option<ResourceRef>,
    umask: Option<u32>,
    budget: BudgetRequest,
}

impl AdmittedExecution {
    /// Whether the instance is long-running or run-to-completion.
    pub const fn kind(&self) -> ExecutionInstanceKind {
        self.kind
    }

    /// The policy selection this instance was admitted under.
    pub const fn policy(&self) -> &ExecutionPolicyFingerprint {
        &self.policy
    }

    /// The effective isolation classes.
    pub fn namespace_classes(&self) -> &[NamespaceClass] {
        &self.namespace_classes
    }

    /// The effective capability set: the required subset inside the ceiling.
    pub fn capability_classes(&self) -> &[CapabilityClass] {
        &self.capability_classes
    }

    /// Whether privilege escalation is disabled for this instance.
    pub const fn no_new_privileges(&self) -> bool {
        self.no_new_privileges
    }

    /// The identity this instance resolves to.
    pub const fn user_ref(&self) -> Option<&ResourceRef> {
        self.user_ref.as_ref()
    }

    /// The syscall filter that must be loaded.
    pub const fn seccomp_profile_ref(&self) -> Option<&ResourceRef> {
        self.seccomp_profile_ref.as_ref()
    }

    /// The admitted umask.
    pub const fn umask(&self) -> Option<u32> {
        self.umask
    }

    /// The admitted limits.
    pub const fn budget(&self) -> &BudgetRequest {
        &self.budget
    }
}

/// One `ExecutionPolicy` contract rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyContractError {
    /// The policy reference named a ResourceType other than `ExecutionPolicy`.
    WrongPolicyResourceType,
    /// The identity reference named a ResourceType other than `User`.
    WrongUserResourceType,
    /// The syscall-filter reference named a ResourceType other than
    /// `SeccompProfile`.
    WrongSeccompResourceType,
    /// A bounded collection exceeded its frozen entry ceiling.
    TooManyEntries,
    /// A numeric field was outside its frozen bound.
    OutOfRange,
}

impl core::fmt::Display for PolicyContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongPolicyResourceType => f.write_str("reference must name an ExecutionPolicy"),
            Self::WrongUserResourceType => f.write_str("identity reference must name a User"),
            Self::WrongSeccompResourceType => f.write_str("filter reference must name a SeccompProfile"),
            Self::TooManyEntries => f.write_str("collection exceeds its frozen bound"),
            Self::OutOfRange => f.write_str("value is outside its frozen bound"),
        }
    }
}

impl std::error::Error for PolicyContractError {}

/// Admit one execution instance against one authorized policy.
///
/// Evaluation is deterministic and stops at the first refusal, so a caller
/// always sees one enforcing stage rather than a partially applied
/// intersection. The result never widens the policy: a required class the
/// policy does not require, a capability outside the ceiling, a weakened
/// mandatory restriction, an unauthorized identity, an incompatible filter,
/// a request over the admitted ceiling, or a mandatory facet the target
/// cannot enforce is refused.
pub fn admit_execution(
    request: &ExecutionInstance,
    requirements: &ExecutionRequirements,
    policy: &ExecutionPolicySpec,
    authorization: &PolicyAuthorization,
    support: &BackendSupport,
    budget_ceiling: &BudgetCeiling,
) -> Result<AdmittedExecution, PolicyRefusal> {
    if !authorization.is_granted() {
        return Err(PolicyRefusal::new(
            AdmissionStage::Authorize,
            RefusalReason::PolicySelectionNotAuthorized,
        ));
    }

    for class in requirements.namespace_classes() {
        if !policy.namespaces().requires(*class) {
            return Err(PolicyRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::RequiredNamespaceNotAdmitted,
            ));
        }
    }
    for class in requirements.capability_classes() {
        if !policy.capabilities().admits(*class) {
            return Err(PolicyRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::RequiredCapabilityOutsideCeiling,
            ));
        }
    }
    if requirements.mandatory_no_new_privileges() && !policy.no_new_privileges() {
        return Err(PolicyRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::RestrictionWeakened,
        ));
    }

    let user_ref = match (policy.identity().user_ref(), request.user_ref()) {
        (None, None) => None,
        (Some(admitted), None) => Some(admitted.clone()),
        (None, Some(_)) => {
            return Err(PolicyRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized,
            ));
        }
        (Some(admitted), Some(requested)) if admitted == requested => Some(admitted.clone()),
        (Some(_), Some(_)) => {
            return Err(PolicyRefusal::new(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized,
            ));
        }
    };
    if policy.identity().requires_user_namespace()
        && !requirements
            .namespace_classes()
            .contains(&NamespaceClass::User)
    {
        return Err(PolicyRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::RequiredNamespaceNotAdmitted,
        ));
    }

    if let Some(required_profile) = requirements.seccomp_profile_ref() {
        match policy.seccomp().profile_ref() {
            Some(admitted) if admitted == required_profile => {}
            _ => {
                return Err(PolicyRefusal::new(
                    AdmissionStage::Admit,
                    RefusalReason::SeccompIncompatible,
                ));
            }
        }
    }

    if !budget_ceiling.admits(request.budget()) {
        return Err(PolicyRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::LimitExceedsCeiling,
        ));
    }

    for facet in policy
        .mandatory_facets()
        .iter()
        .chain(requirements.mandatory_facets())
    {
        if !support.enforces(*facet) {
            return Err(PolicyRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::MandatoryFacetUnsupported,
            ));
        }
    }

    let mut namespace_classes = requirements.namespace_classes().to_vec();
    namespace_classes.sort_unstable();
    namespace_classes.dedup();
    for class in policy.namespaces().classes() {
        if !namespace_classes.contains(class) {
            namespace_classes.push(*class);
        }
    }

    let mut capability_classes = requirements.capability_classes().to_vec();
    capability_classes.sort_unstable();
    capability_classes.dedup();

    Ok(AdmittedExecution {
        kind: request.kind(),
        policy: ExecutionPolicyFingerprint::from_spec(policy),
        namespace_classes,
        capability_classes,
        // Instance input may tighten this but never weaken it.
        no_new_privileges: policy.no_new_privileges() || requirements.mandatory_no_new_privileges(),
        user_ref,
        seccomp_profile_ref: policy.seccomp().profile_ref().cloned(),
        umask: policy.umask(),
        budget: *request.budget(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::process::CapabilityClass;


    fn user() -> ResourceRef {
        ResourceRef::parse("User/alice").expect("user ref")
    }

    fn profile() -> ResourceRef {
        ResourceRef::parse("SeccompProfile/desktop").expect("profile ref")
    }

    fn policy(
        namespaces: Vec<NamespaceClass>,
        capabilities: Vec<CapabilityClass>,
        identity: Option<ResourceRef>,
        no_new_privileges: bool,
        seccomp: Option<ResourceRef>,
    ) -> ExecutionPolicySpec {
        ExecutionPolicySpec::new(
            PolicyNamespaces::new(namespaces).expect("namespaces"),
            PolicyCapabilities::new(capabilities).expect("capabilities"),
            no_new_privileges,
            PolicyIdentity::new(identity, false).expect("identity"),
            PolicyRoot::new(true, true),
            PolicySeccomp::new(seccomp).expect("seccomp"),
            Some(0o077),
        )
        .expect("policy")
    }

    fn request(kind: ExecutionInstanceKind) -> ExecutionInstance {
        ExecutionInstance::new(kind, None, BudgetRequest::new(500, 1024 * 1024, 32, 64).expect("budget"))
            .expect("instance")
    }

    fn requirements() -> ExecutionRequirements {
        ExecutionRequirements::new(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            false,
            None,
            Vec::new(),
        )
        .expect("requirements")
    }

    fn support() -> BackendSupport {
        BackendSupport::new(vec![
            ConfinementFacet::UserNamespace,
            ConfinementFacet::MountNamespace,
            ConfinementFacet::ReadOnlyRoot,
            ConfinementFacet::PrivateRoot,
            ConfinementFacet::CapabilityCeiling,
            ConfinementFacet::NoNewPrivileges,
        ])
        .expect("support")
    }

    fn ceiling() -> BudgetCeiling {
        BudgetCeiling::new(1_000, 4 * 1024 * 1024, 64, 128)
    }


    fn collect_keys(value: &serde_json::Value, keys: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, nested) in map {
                    keys.push(key.clone());
                    collect_keys(nested, keys);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_keys(item, keys);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn a_policy_declares_no_resource_access() {
        let policy = policy(
            vec![NamespaceClass::Mount],
            Vec::new(),
            None,
            true,
            None,
        );
        let bytes = crate::v3::resource_schema::canonical_json_bytes(&policy).expect("canonical");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        let mut keys = Vec::new();
        collect_keys(&json, &mut keys);
        for key in &keys {
            let lowered = key.to_ascii_lowercase();
            for forbidden in [
                "volume",
                "device",
                "network",
                "endpoint",
                "credential",
                "mount",
                "path",
                "hostpath",
            ] {
                assert!(
                    !lowered.contains(forbidden),
                    "{key} is not a policy field: a policy declares confinement, not access"
                );
            }
        }
        assert!(keys.contains(&"namespaces".to_owned()));
        assert!(keys.contains(&"noNewPrivileges".to_owned()));
    }

    #[test]
    fn selecting_a_policy_without_authorization_is_refused() {
        let policy = policy(vec![NamespaceClass::Mount], Vec::new(), None, false, None);
        let refusal = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &requirements(),
            &policy,
            &PolicyAuthorization::absent(),
            &support(),
            &ceiling(),
        )
        .expect_err("an unauthorized selection must be refused");
        assert_eq!(refusal.reason(), RefusalReason::PolicySelectionNotAuthorized);
        assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    }

    #[test]
    fn a_missing_required_namespace_is_refused_not_intersected() {
        let policy = policy(vec![NamespaceClass::Mount], Vec::new(), None, false, None);
        let refusal = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &requirements(),
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("a required class outside the policy must be refused");
        assert_eq!(
            refusal.reason(),
            RefusalReason::RequiredNamespaceNotAdmitted
        );
    }

    #[test]
    fn a_required_capability_outside_the_ceiling_is_refused() {
        let policy = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::SysTime],
            None,
            false,
            None,
        );
        let refusal = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &requirements(),
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("a capability outside the ceiling must be refused");
        assert_eq!(
            refusal.reason(),
            RefusalReason::RequiredCapabilityOutsideCeiling
        );
    }

    #[test]
    fn a_mandatory_restriction_is_never_weakened_by_instance_input() {
        let permissive = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            None,
            false,
            None,
        );
        let strict_requirements = ExecutionRequirements::new(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            true,
            None,
            Vec::new(),
        )
        .expect("requirements");
        let refusal = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &strict_requirements,
            &permissive,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("an implementation that needs the restriction must be refused");
        assert_eq!(refusal.reason(), RefusalReason::RestrictionWeakened);

        let strict = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            None,
            true,
            None,
        );
        let admitted = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &requirements(),
            &strict,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect("admitted");
        assert!(admitted.no_new_privileges());
    }

    #[test]
    fn identity_is_resolved_only_through_the_authorized_rule() {
        let policy = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            Some(user()),
            false,
            None,
        );
        let foreign = ExecutionInstance::new(
            ExecutionInstanceKind::LongRunning,
            Some(ResourceRef::parse("User/bob").expect("user ref")),
            BudgetRequest::new(1, 1, 1, 1).expect("budget"),
        )
        .expect("instance");
        let refusal = admit_execution(
            &foreign,
            &requirements(),
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("an unauthorized identity must be refused");
        assert_eq!(refusal.reason(), RefusalReason::IdentityNotAuthorized);

        let admitted = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &requirements(),
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect("admitted");
        assert_eq!(admitted.user_ref(), Some(&user()));
    }

    #[test]
    fn an_incompatible_syscall_filter_is_refused() {
        let policy = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            None,
            false,
            Some(profile()),
        );
        let needs_other = ExecutionRequirements::new(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            false,
            Some(ResourceRef::parse("SeccompProfile/other").expect("profile")),
            Vec::new(),
        )
        .expect("requirements");
        let refusal = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &needs_other,
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("an incompatible filter must be refused");
        assert_eq!(refusal.reason(), RefusalReason::SeccompIncompatible);
    }

    #[test]
    fn a_request_over_the_admitted_ceiling_is_refused() {
        let policy = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            None,
            false,
            None,
        );
        let oversubscribed = ExecutionInstance::new(
            ExecutionInstanceKind::LongRunning,
            None,
            BudgetRequest::new(2_000, 1, 1, 1).expect("budget"),
        )
        .expect("instance");
        let refusal = admit_execution(
            &oversubscribed,
            &requirements(),
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("an over-ceiling request must be refused");
        assert_eq!(refusal.reason(), RefusalReason::LimitExceedsCeiling);
    }

    #[test]
    fn a_mandatory_facet_the_target_cannot_enforce_is_refused() {
        let policy = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount, NamespaceClass::Network],
            vec![CapabilityClass::NetworkBind],
            None,
            false,
            None,
        );
        let refusal = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &requirements(),
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect_err("an unenforceable mandatory facet must be refused");
        assert_eq!(refusal.reason(), RefusalReason::MandatoryFacetUnsupported);
    }

    #[test]
    fn an_admitted_execution_never_widens_the_policy() {
        let policy = policy(
            vec![NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind, CapabilityClass::SysTime],
            None,
            false,
            None,
        );
        let fewer = ExecutionRequirements::new(
            vec![NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            false,
            None,
            Vec::new(),
        )
        .expect("requirements");
        let admitted = admit_execution(
            &request(ExecutionInstanceKind::LongRunning),
            &fewer,
            &policy,
            &PolicyAuthorization::granted(),
            &support(),
            &ceiling(),
        )
        .expect("admitted");
        assert_eq!(
            admitted.capability_classes(),
            &[CapabilityClass::NetworkBind],
            "an admitted capability outside the request is not granted"
        );
        assert_eq!(admitted.namespace_classes(), &[NamespaceClass::Mount]);
    }

    #[test]
    fn a_one_shot_and_a_long_running_instance_take_the_same_path() {
        let policy = policy(
            vec![NamespaceClass::User, NamespaceClass::Mount],
            vec![CapabilityClass::NetworkBind],
            None,
            true,
            None,
        );
        let admitted = [
            ExecutionInstanceKind::LongRunning,
            ExecutionInstanceKind::OneShot,
        ]
        .map(|kind| {
            admit_execution(
                &request(kind),
                &requirements(),
                &policy,
                &PolicyAuthorization::granted(),
                &support(),
                &ceiling(),
            )
            .expect("admitted")
        });
        assert_eq!(admitted[0].kind(), ExecutionInstanceKind::LongRunning);
        assert_eq!(admitted[1].kind(), ExecutionInstanceKind::OneShot);
        assert_eq!(
            admitted[0].namespace_classes(),
            admitted[1].namespace_classes()
        );
        assert_eq!(
            admitted[0].capability_classes(),
            admitted[1].capability_classes()
        );
        assert_eq!(
            admitted[0].no_new_privileges(),
            admitted[1].no_new_privileges()
        );
        assert_eq!(admitted[0].umask(), admitted[1].umask());
    }
}
