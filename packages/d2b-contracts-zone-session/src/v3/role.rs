//! Native RBAC Role contract.
//!
//! Resource and ComponentSession verbs are intentionally separate closed
//! sets.  In particular, `relay` is transport forwarding authority and can
//! never be smuggled into CRUD by treating all verbs as strings.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_resource::v3::{ ResourceName, ResourceRef, ResourceTypeName, ZoneId, execution_policy::{
        BoundedText, BoundedToken, MAX_PATH_BYTES, parsed_deserialize, redacted_debug,
        require_resource_type,
    } };

/// Frozen wire role vocabulary: the Role contract names the resources its
/// authority facets may reference. These are the canonical ResourceType
/// names of the ownership-declared authority, restated here because the
/// zone-session contract is frozen wire and must not depend on the owning
/// provider crates (their own crates depend on the declaration layer that
/// reaches back to this contract). The generated type authority in
/// `d2b_contracts::identity::STANDARD_RESOURCE_TYPES` pins the same names;
/// a rename there cannot happen without a concurrent wire change.
const COMMAND_RESOURCE_TYPE: &str = "Command";
/// Frozen wire role vocabulary: the canonical Operation ResourceType name.
const OPERATION_RESOURCE_TYPE: &str = "Operation";
/// Frozen wire role vocabulary: the canonical SeccompProfile ResourceType name.
const SECCOMP_PROFILE_RESOURCE_TYPE: &str = "SeccompProfile";

/// Canonical Role ResourceType name.
pub const ROLE_RESOURCE_TYPE: &str = "Role";
/// Maximum rules in one Role.
pub const MAX_ROLE_RULES: usize = 32;
/// Maximum ResourceTypes in one rule.
pub const MAX_ROLE_RULE_RESOURCE_TYPES: usize = 16;
/// Maximum resource verbs in one rule.
pub const MAX_ROLE_RULE_VERBS: usize = 16;
/// Maximum session verbs in one rule.
pub const MAX_ROLE_RULE_SESSION_VERBS: usize = 9;
/// Maximum subresource selectors in one rule.
pub const MAX_ROLE_RULE_SUBRESOURCES: usize = 16;
/// Maximum resource-name selectors in one rule.
pub const MAX_ROLE_RULE_RESOURCE_NAMES: usize = 64;
/// Maximum execution references in one rule.
pub const MAX_ROLE_RULE_EXECUTION_REFS: usize = 32;
/// Maximum Zone selectors in one rule.
pub const MAX_ROLE_RULE_ZONES: usize = 8;
/// Maximum operation references in one Role.
pub const MAX_ROLE_OPERATION_REFS: usize = 64;
/// Maximum command references in one Role.
pub const MAX_ROLE_COMMAND_REFS: usize = 64;
/// Maximum capabilities in one Role posture.
pub const MAX_ROLE_POSTURE_CAPABILITIES: usize = 64;
/// Maximum mounts in one Role posture.
pub const MAX_ROLE_POSTURE_MOUNTS: usize = 64;
/// Highest umask a Role posture may request.
pub const MAX_ROLE_POSTURE_UMASK: u32 = 0o777;
/// Maximum bytes of the host-account name part of a `Principal` reference.
pub const MAX_PRINCIPAL_NAME_BYTES: usize = 63;
/// Canonical `Principal` reference prefix.
const PRINCIPAL_REF_PREFIX: &str = "Principal/";
/// Canonical `Principal/<name>` reference spelling.
const PRINCIPAL_REF_PATTERN: &str = "^Principal/[a-z][a-z0-9-]{0,62}$";
/// Absolute mount-path spelling. The compiled rule also refuses control
/// characters.
const MOUNT_PATH_PATTERN: &str = "^/[^\\u0000]*$";
/// Core finalizer used while RoleBindings drain.
pub const ROLE_BINDING_DRAIN_FINALIZER: &str = "core.role-binding-drain";

/// Closed resource authorization verbs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleResourceVerb {
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

impl RoleResourceVerb {
    /// Every resource verb in stable order.
    pub const ALL: [Self; 11] = [
        Self::Get,
        Self::List,
        Self::Watch,
        Self::Create,
        Self::UpdateSpec,
        Self::UpdateStatus,
        Self::UpdateMetadata,
        Self::UpdateFinalizers,
        Self::Delete,
        Self::UseCredential,
        Self::AdminCredential,
    ];
}

/// Closed ComponentSession authorization verbs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleSessionVerb {
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

impl RoleSessionVerb {
    /// Every session verb in stable order.
    pub const ALL: [Self; 9] = [
        Self::Connect,
        Self::Invoke,
        Self::OpenStream,
        Self::Relay,
        Self::Attach,
        Self::Cancel,
        Self::Observe,
        Self::AuditExport,
        Self::SupportBundle,
    ];
}

/// Role validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleContractError {
    EmptyResourceTypes,
    EmptyVerbs,
    BoundExceeded,
    DuplicateEntry,
    InvalidResourceName,
    InvalidExecutionRef,
    InvalidCredentialScope,
    RelayScopeRequired,
    RelayHasResourceVerb,
    RelaySelectorInvalid,
    DiagnosticSelectorInvalid,
    WildcardNotAllowed,
    InvalidWildcard,
    InvalidOperationRef,
    InvalidCommandRef,
    InvalidSeccompRef,
    InvalidPrincipalRef,
    InvalidMountPath,
    InvalidUmask,
}

impl core::fmt::Display for RoleContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyResourceTypes => "role-resource-types-empty",
            Self::EmptyVerbs => "role-verbs-empty",
            Self::BoundExceeded => "role-bound-exceeded",
            Self::DuplicateEntry => "role-duplicate-entry",
            Self::InvalidResourceName => "role-resource-name-invalid",
            Self::InvalidExecutionRef => "role-execution-ref-invalid",
            Self::InvalidCredentialScope => "role-credential-scope-invalid",
            Self::RelayScopeRequired => "role-relay-scope-required",
            Self::RelayHasResourceVerb => "role-relay-resource-verb",
            Self::RelaySelectorInvalid => "role-relay-selector-invalid",
            Self::DiagnosticSelectorInvalid => "role-diagnostic-selector-invalid",
            Self::WildcardNotAllowed => "role-wildcard-not-allowed",
            Self::InvalidWildcard => "role-wildcard-invalid",
            Self::InvalidOperationRef => "role-operation-ref-invalid",
            Self::InvalidCommandRef => "role-command-ref-invalid",
            Self::InvalidSeccompRef => "role-seccomp-ref-invalid",
            Self::InvalidPrincipalRef => "role-principal-ref-invalid",
            Self::InvalidMountPath => "role-mount-path-invalid",
            Self::InvalidUmask => "role-umask-invalid",
        })
    }
}

impl std::error::Error for RoleContractError {}

/// One exact Role rule.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleRule {
    resource_types: Vec<ResourceTypeName>,
    verbs: Vec<RoleResourceVerb>,
    subresources: Vec<BoundedText>,
    resource_names: Vec<String>,
    zones: Vec<ZoneId>,
    execution_refs: Vec<ResourceRef>,
    session_verbs: Vec<RoleSessionVerb>,
}

impl RoleRule {
    /// Construct and canonicalize one Role rule.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut resource_types: Vec<ResourceTypeName>,
        mut verbs: Vec<RoleResourceVerb>,
        mut subresources: Vec<BoundedText>,
        mut resource_names: Vec<String>,
        mut zones: Vec<ZoneId>,
        mut execution_refs: Vec<ResourceRef>,
        mut session_verbs: Vec<RoleSessionVerb>,
    ) -> Result<Self, RoleContractError> {
        if resource_types.is_empty() {
            return Err(RoleContractError::EmptyResourceTypes);
        }
        if verbs.is_empty() && session_verbs.is_empty() {
            return Err(RoleContractError::EmptyVerbs);
        }
        if resource_types.len() > MAX_ROLE_RULE_RESOURCE_TYPES
            || verbs.len() > MAX_ROLE_RULE_VERBS
            || session_verbs.len() > MAX_ROLE_RULE_SESSION_VERBS
            || subresources.len() > MAX_ROLE_RULE_SUBRESOURCES
            || resource_names.len() > MAX_ROLE_RULE_RESOURCE_NAMES
            || zones.len() > MAX_ROLE_RULE_ZONES
            || execution_refs.len() > MAX_ROLE_RULE_EXECUTION_REFS
        {
            return Err(RoleContractError::BoundExceeded);
        }
        resource_types.sort();
        verbs.sort();
        subresources.sort();
        zones.sort();
        execution_refs.sort();
        session_verbs.sort();
        if duplicate(&resource_types)
            || duplicate(&verbs)
            || duplicate(&subresources)
            || duplicate(&zones)
            || duplicate(&execution_refs)
            || duplicate(&session_verbs)
        {
            return Err(RoleContractError::DuplicateEntry);
        }
        for name in &resource_names {
            if name == "*" {
                continue;
            }
            ResourceName::parse(name.clone())
                .map_err(|_| RoleContractError::InvalidResourceName)?;
        }
        resource_names.sort();
        if duplicate(&resource_names) {
            return Err(RoleContractError::DuplicateEntry);
        }
        if execution_refs
            .iter()
            .any(|reference| reference.resource_type().as_str() == "ZoneLink")
        {
            // ZoneLink refs are valid execution selectors only when a caller
            // explicitly binds them; the type itself remains canonical.
        }
        if execution_refs.iter().any(|reference| {
            !matches!(
                reference.resource_type().as_str(),
                "Host" | "Guest" | "Process" | "ZoneLink"
            )
        }) {
            return Err(RoleContractError::InvalidExecutionRef);
        }
        let has_relay = session_verbs.contains(&RoleSessionVerb::Relay);
        if has_relay {
            if !verbs.is_empty() || resource_names.is_empty() || zones.is_empty() {
                return Err(if verbs.is_empty() {
                    RoleContractError::RelayScopeRequired
                } else {
                    RoleContractError::RelayHasResourceVerb
                });
            }
            if resource_names.iter().any(|name| name == "*") {
                return Err(RoleContractError::RelaySelectorInvalid);
            }
        }
        let has_diagnostic = session_verbs.iter().any(|verb| {
            matches!(
                verb,
                RoleSessionVerb::AuditExport | RoleSessionVerb::SupportBundle
            )
        });
        if has_diagnostic
            && (subresources.is_empty()
                || subresources.iter().any(|selector| {
                    !matches!(
                        selector.as_str(),
                        "d2b.audit.v3.AuditService/Export"
                            | "d2b.support.v3.SupportService/GenerateBundle"
                    )
                }))
        {
            return Err(RoleContractError::DiagnosticSelectorInvalid);
        }
        if verbs.iter().any(|verb| {
            matches!(
                verb,
                RoleResourceVerb::UseCredential | RoleResourceVerb::AdminCredential
            )
        }) && (resource_types.len() != 1
            || resource_types[0].as_str() != "Credential"
            || subresources.is_empty())
        {
            return Err(RoleContractError::InvalidCredentialScope);
        }
        if verbs.contains(&RoleResourceVerb::AdminCredential)
            && (subresources
                .iter()
                .any(|selector| !matches!(selector.as_str(), "create" | "update-spec" | "delete"))
                || subresources.iter().any(|selector| {
                    let required = match selector.as_str() {
                        "create" => RoleResourceVerb::Create,
                        "update-spec" => RoleResourceVerb::UpdateSpec,
                        "delete" => RoleResourceVerb::Delete,
                        _ => return true,
                    };
                    !verbs.contains(&required)
                }))
        {
            return Err(RoleContractError::InvalidCredentialScope);
        }
        Ok(Self {
            resource_types,
            verbs,
            subresources,
            resource_names,
            zones,
            execution_refs,
            session_verbs,
        })
    }

    /// Validate provenance-dependent wildcard rules.
    pub fn validate_provenance(
        &self,
        core_controller_generated: bool,
    ) -> Result<(), RoleContractError> {
        let wildcard_count = self
            .resource_names
            .iter()
            .filter(|name| name.as_str() == "*")
            .count();
        if wildcard_count > 1 {
            return Err(RoleContractError::InvalidWildcard);
        }
        if wildcard_count != 0 && !core_controller_generated {
            return Err(RoleContractError::WildcardNotAllowed);
        }
        Ok(())
    }

    /// Borrow ResourceTypes.
    pub fn resource_types(&self) -> &[ResourceTypeName] {
        &self.resource_types
    }

    /// Borrow resource verbs.
    pub fn verbs(&self) -> &[RoleResourceVerb] {
        &self.verbs
    }

    /// Borrow service/subresource selectors.
    pub fn subresources(&self) -> &[BoundedText] {
        &self.subresources
    }

    /// Borrow exact name selectors. `"*"` is only valid for reviewed core
    /// roles and is never an implicit wildcard.
    pub fn resource_names(&self) -> &[String] {
        &self.resource_names
    }

    /// Borrow Zone selectors.
    pub fn zones(&self) -> &[ZoneId] {
        &self.zones
    }

    /// Borrow execution selectors.
    pub fn execution_refs(&self) -> &[ResourceRef] {
        &self.execution_refs
    }

    /// Borrow session verbs.
    pub fn session_verbs(&self) -> &[RoleSessionVerb] {
        &self.session_verbs
    }

    /// Whether this rule contains relay authority.
    pub fn permits_relay(&self) -> bool {
        self.session_verbs.contains(&RoleSessionVerb::Relay)
    }
}

redacted_debug!(RoleRule);

impl<'de> Deserialize<'de> for RoleRule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            resource_types: Vec<ResourceTypeName>,
            #[serde(default)]
            verbs: Vec<RoleResourceVerb>,
            #[serde(default)]
            subresources: Vec<BoundedText>,
            #[serde(default)]
            resource_names: Vec<String>,
            #[serde(default)]
            zones: Vec<ZoneId>,
            #[serde(default)]
            execution_refs: Vec<ResourceRef>,
            #[serde(default)]
            session_verbs: Vec<RoleSessionVerb>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.resource_types,
            wire.verbs,
            wire.subresources,
            wire.resource_names,
            wire.zones,
            wire.execution_refs,
            wire.session_verbs,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn duplicate<T: PartialEq>(values: &[T]) -> bool {
    values.windows(2).any(|pair| pair[0] == pair[1])
}

/// A validated `Principal/<name>` host-account reference.
///
/// A principal names a host account allocated by the committed principal
/// allocation. It is deliberately not a `ResourceRef`: no resource selector
/// may name a host account, and no resource may forge one.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PrincipalRef(String);

impl PrincipalRef {
    /// Parse `^Principal/[a-z][a-z0-9-]{0,62}$`.
    pub fn parse(value: impl Into<String>) -> Result<Self, RoleContractError> {
        let value = value.into();
        if let Some(name) = value.strip_prefix(PRINCIPAL_REF_PREFIX)
            && !name.is_empty()
            && name.len() <= MAX_PRINCIPAL_NAME_BYTES
            && name.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_lowercase()
                } else {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                }
            })
        {
            return Ok(Self(value));
        }
        Err(RoleContractError::InvalidPrincipalRef)
    }

    /// Borrow the host-account name after the `Principal/` prefix.
    pub fn name(&self) -> &str {
        &self.0[PRINCIPAL_REF_PREFIX.len()..]
    }
}

redacted_debug!(PrincipalRef);
parsed_deserialize!(PrincipalRef);

impl JsonSchema for PrincipalRef {
    fn schema_name() -> String {
        "PrincipalRef".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().pattern = Some(PRINCIPAL_REF_PATTERN.to_owned());
        schema.string().max_length =
            Some((PRINCIPAL_REF_PREFIX.len() + MAX_PRINCIPAL_NAME_BYTES) as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// A validated absolute posture mount path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RoleMountPath(String);

impl RoleMountPath {
    /// Parse an absolute path bounded to 255 bytes with no control characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, RoleContractError> {
        let value = value.into();
        if !value.starts_with('/')
            || value.len() > MAX_PATH_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(RoleContractError::InvalidMountPath);
        }
        Ok(Self(value))
    }

    /// Borrow the path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(RoleMountPath);
parsed_deserialize!(RoleMountPath);

impl JsonSchema for RoleMountPath {
    fn schema_name() -> String {
        "RoleMountPath".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().pattern = Some(MOUNT_PATH_PATTERN.to_owned());
        schema.string().max_length = Some(MAX_PATH_BYTES as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// One mount a posture grants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleMount {
    path: RoleMountPath,
    #[serde(default)]
    writable: bool,
}

impl RoleMount {
    /// Construct one mount.
    pub const fn new(path: RoleMountPath, writable: bool) -> Self {
        Self { path, writable }
    }

    /// Borrow the absolute mount path.
    pub const fn path(&self) -> &RoleMountPath {
        &self.path
    }

    /// Whether the mount is writable.
    pub const fn writable(&self) -> bool {
        self.writable
    }
}

/// The namespace set one posture isolates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleNamespaces {
    #[serde(default)]
    mount: bool,
    #[serde(default)]
    pid: bool,
    #[serde(default)]
    net: bool,
    #[serde(default)]
    uts: bool,
    #[serde(default)]
    ipc: bool,
    #[serde(default)]
    cgroup: bool,
    #[serde(default)]
    time: bool,
}

impl RoleNamespaces {
    /// Construct one namespace set.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        mount: bool,
        pid: bool,
        net: bool,
        uts: bool,
        ipc: bool,
        cgroup: bool,
        time: bool,
    ) -> Self {
        Self {
            mount,
            pid,
            net,
            uts,
            ipc,
            cgroup,
            time,
        }
    }

    /// Whether a mount namespace is isolated.
    pub const fn mount(&self) -> bool {
        self.mount
    }

    /// Whether a PID namespace is isolated.
    pub const fn pid(&self) -> bool {
        self.pid
    }

    /// Whether a network namespace is isolated.
    pub const fn net(&self) -> bool {
        self.net
    }

    /// Whether a UTS namespace is isolated.
    pub const fn uts(&self) -> bool {
        self.uts
    }

    /// Whether an IPC namespace is isolated.
    pub const fn ipc(&self) -> bool {
        self.ipc
    }

    /// Whether a cgroup namespace is isolated.
    pub const fn cgroup(&self) -> bool {
        self.cgroup
    }

    /// Whether a time namespace is isolated.
    pub const fn time(&self) -> bool {
        self.time
    }
}

/// The confined posture one Role grants its Processes.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolePosture {
    seccomp_ref: ResourceRef,
    principal_ref: PrincipalRef,
    capabilities: Vec<BoundedToken>,
    namespaces: RoleNamespaces,
    mounts: Vec<RoleMount>,
    umask: Option<u32>,
    user_ns: bool,
}

impl RolePosture {
    /// Construct a posture after checking every reference, bound, and umask.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seccomp_ref: ResourceRef,
        principal_ref: PrincipalRef,
        capabilities: Vec<BoundedToken>,
        namespaces: RoleNamespaces,
        mounts: Vec<RoleMount>,
        umask: Option<u32>,
        user_ns: bool,
    ) -> Result<Self, RoleContractError> {
        require_resource_type(&seccomp_ref, SECCOMP_PROFILE_RESOURCE_TYPE)
            .map_err(|_| RoleContractError::InvalidSeccompRef)?;
        if capabilities.len() > MAX_ROLE_POSTURE_CAPABILITIES
            || mounts.len() > MAX_ROLE_POSTURE_MOUNTS
        {
            return Err(RoleContractError::BoundExceeded);
        }
        if umask.is_some_and(|value| value > MAX_ROLE_POSTURE_UMASK) {
            return Err(RoleContractError::InvalidUmask);
        }
        Ok(Self {
            seccomp_ref,
            principal_ref,
            capabilities,
            namespaces,
            mounts,
            umask,
            user_ns,
        })
    }

    /// Borrow the referenced SeccompProfile.
    pub const fn seccomp_ref(&self) -> &ResourceRef {
        &self.seccomp_ref
    }

    /// Borrow the host-account principal this posture runs as.
    pub const fn principal_ref(&self) -> &PrincipalRef {
        &self.principal_ref
    }

    /// Borrow the capability tokens.
    pub fn capabilities(&self) -> &[BoundedToken] {
        &self.capabilities
    }

    /// Borrow the namespace set.
    pub const fn namespaces(&self) -> &RoleNamespaces {
        &self.namespaces
    }

    /// Borrow the mounts.
    pub fn mounts(&self) -> &[RoleMount] {
        &self.mounts
    }

    /// The requested umask, when the posture pins one.
    pub const fn umask(&self) -> Option<u32> {
        self.umask
    }

    /// Whether a user namespace is isolated.
    pub const fn user_ns(&self) -> bool {
        self.user_ns
    }
}

redacted_debug!(RolePosture);

impl<'de> Deserialize<'de> for RolePosture {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            seccomp_ref: ResourceRef,
            principal_ref: PrincipalRef,
            #[serde(default)]
            capabilities: Vec<BoundedToken>,
            #[serde(default)]
            namespaces: RoleNamespaces,
            #[serde(default)]
            mounts: Vec<RoleMount>,
            #[serde(default)]
            umask: Option<u32>,
            #[serde(default)]
            user_ns: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.seccomp_ref,
            wire.principal_ref,
            wire.capabilities,
            wire.namespaces,
            wire.mounts,
            wire.umask,
            wire.user_ns,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The complete Role desired state.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleSpec {
    rules: Vec<RoleRule>,
    operation_refs: Vec<ResourceRef>,
    command_refs: Vec<ResourceRef>,
    posture: Option<RolePosture>,
}

impl RoleSpec {
    /// Construct a bounded Role spec with an empty authority and posture
    /// facet.
    pub fn new(rules: Vec<RoleRule>) -> Result<Self, RoleContractError> {
        Self::with_facets(rules, Vec::new(), Vec::new(), None)
    }

    /// Construct a bounded Role spec with its authority and posture facets.
    pub fn with_facets(
        rules: Vec<RoleRule>,
        operation_refs: Vec<ResourceRef>,
        command_refs: Vec<ResourceRef>,
        posture: Option<RolePosture>,
    ) -> Result<Self, RoleContractError> {
        if rules.is_empty()
            || rules.len() > MAX_ROLE_RULES
            || operation_refs.len() > MAX_ROLE_OPERATION_REFS
            || command_refs.len() > MAX_ROLE_COMMAND_REFS
        {
            return Err(RoleContractError::BoundExceeded);
        }
        for reference in &operation_refs {
            require_resource_type(reference, OPERATION_RESOURCE_TYPE)
                .map_err(|_| RoleContractError::InvalidOperationRef)?;
        }
        for reference in &command_refs {
            require_resource_type(reference, COMMAND_RESOURCE_TYPE)
                .map_err(|_| RoleContractError::InvalidCommandRef)?;
        }
        Ok(Self {
            rules,
            operation_refs,
            command_refs,
            posture,
        })
    }

    /// Borrow rules.
    pub fn rules(&self) -> &[RoleRule] {
        &self.rules
    }

    /// Borrow the declared Operations this Role may launch.
    pub fn operation_refs(&self) -> &[ResourceRef] {
        &self.operation_refs
    }

    /// Borrow the declared Commands this Role may launch.
    pub fn command_refs(&self) -> &[ResourceRef] {
        &self.command_refs
    }

    /// Borrow the optional confined posture.
    pub fn posture(&self) -> Option<&RolePosture> {
        self.posture.as_ref()
    }
}

redacted_debug!(RoleSpec);

impl<'de> Deserialize<'de> for RoleSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            rules: Vec<RoleRule>,
            #[serde(default)]
            operation_refs: Vec<ResourceRef>,
            #[serde(default)]
            command_refs: Vec<ResourceRef>,
            #[serde(default)]
            posture: Option<RolePosture>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::with_facets(
            wire.rules,
            wire.operation_refs,
            wire.command_refs,
            wire.posture,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Closed Role condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RoleConditionType {
    RuleSetValid,
    IndexBuilt,
    ActiveBindings,
    PendingBindingDrain,
}

/// Validate a Role owner reference.
pub fn validate_role_owner(owner: Option<&ResourceRef>) -> Result<(), RoleContractError> {
    if owner.is_some_and(|reference| {
        !matches!(reference.resource_type().as_str(), "Provider" | "ZoneLink")
    }) {
        Err(RoleContractError::InvalidExecutionRef)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::CanonicalJsonObject;

    fn type_name() -> ResourceTypeName {
        ResourceTypeName::parse("Process").unwrap()
    }

    #[test]
    fn resource_and_session_verbs_are_separate() {
        let rule = RoleRule::new(
            vec![type_name()],
            vec![RoleResourceVerb::Get],
            Vec::new(),
            vec![String::from("worker")],
            vec![ZoneId::parse("dev").unwrap()],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(!rule.permits_relay());
        assert_eq!(rule.verbs(), &[RoleResourceVerb::Get]);
    }

    #[test]
    fn relay_requires_bounded_nonempty_scope_and_never_crud() {
        assert!(
            RoleRule::new(
                vec![type_name()],
                vec![],
                vec![],
                vec!["worker".to_owned()],
                vec![ZoneId::parse("dev").unwrap()],
                vec![],
                vec![RoleSessionVerb::Relay],
            )
            .unwrap()
            .permits_relay()
        );
        assert!(
            RoleRule::new(
                vec![type_name()],
                vec![RoleResourceVerb::Get],
                vec![],
                vec!["worker".to_owned()],
                vec![ZoneId::parse("dev").unwrap()],
                vec![],
                vec![RoleSessionVerb::Relay],
            )
            .is_err()
        );
    }

    #[test]
    fn wildcard_is_explicit_and_provenance_bound() {
        let rule = RoleRule::new(
            vec![type_name()],
            vec![RoleResourceVerb::Get],
            vec![],
            vec!["*".to_owned()],
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        assert!(rule.validate_provenance(true).is_ok());
        assert_eq!(
            rule.validate_provenance(false),
            Err(RoleContractError::WildcardNotAllowed)
        );
    }

    fn process_rule() -> RoleRule {
        RoleRule::new(
            vec![type_name()],
            vec![RoleResourceVerb::Get],
            Vec::new(),
            vec!["worker".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn posture_with(
        capabilities: Vec<BoundedToken>,
        mounts: Vec<RoleMount>,
        umask: Option<u32>,
    ) -> Result<RolePosture, RoleContractError> {
        RolePosture::new(
            ResourceRef::parse("SeccompProfile/worker").unwrap(),
            PrincipalRef::parse("Principal/worker").unwrap(),
            capabilities,
            RoleNamespaces::default(),
            mounts,
            umask,
            false,
        )
    }

    #[test]
    fn operation_and_command_refs_are_type_gated() {
        assert_eq!(
            RoleSpec::with_facets(
                vec![process_rule()],
                vec![ResourceRef::parse("Process/worker").unwrap()],
                Vec::new(),
                None,
            ),
            Err(RoleContractError::InvalidOperationRef)
        );
        assert_eq!(
            RoleSpec::with_facets(
                vec![process_rule()],
                vec![ResourceRef::parse("Operation/start").unwrap()],
                vec![ResourceRef::parse("Operation/start").unwrap()],
                None,
            ),
            Err(RoleContractError::InvalidCommandRef)
        );
        let spec = RoleSpec::with_facets(
            vec![process_rule()],
            vec![ResourceRef::parse("Operation/start").unwrap()],
            vec![ResourceRef::parse("Command/worker").unwrap()],
            None,
        )
        .unwrap();
        assert_eq!(spec.operation_refs().len(), 1);
        assert_eq!(spec.command_refs().len(), 1);
        assert_eq!(
            serde_json::from_str::<RoleSpec>(
                r#"{"rules":[{"resourceTypes":["Process"],"verbs":["get"],"subresources":[],"resourceNames":["worker"],"zones":[],"executionRefs":[],"sessionVerbs":[]}],"operationRefs":["Process/worker"]}"#
            )
            .unwrap_err()
            .to_string()
            .split(" at line")
            .next(),
            Some("role-operation-ref-invalid")
        );
    }

    #[test]
    fn principal_refs_and_seccomp_refs_are_gated() {
        for value in [
            "Principal/".to_owned(),
            "Principal/Upper".to_owned(),
            "Principal/with_underscore".to_owned(),
            "principal/worker".to_owned(),
            "User/alice".to_owned(),
            format!("Principal/{}", "z".repeat(MAX_PRINCIPAL_NAME_BYTES + 1)),
        ] {
            assert_eq!(
                PrincipalRef::parse(value),
                Err(RoleContractError::InvalidPrincipalRef)
            );
        }
        let principal = PrincipalRef::parse("Principal/d2bd").unwrap();
        assert_eq!(principal.name(), "d2bd");
        assert_eq!(
            serde_json::to_string(&principal).unwrap(),
            "\"Principal/d2bd\""
        );
        assert_eq!(
            RolePosture::new(
                ResourceRef::parse("Process/worker").unwrap(),
                PrincipalRef::parse("Principal/worker").unwrap(),
                Vec::new(),
                RoleNamespaces::default(),
                Vec::new(),
                None,
                false,
            ),
            Err(RoleContractError::InvalidSeccompRef)
        );
    }

    #[test]
    fn posture_umask_over_511_is_refused() {
        assert!(posture_with(Vec::new(), Vec::new(), Some(0o777)).is_ok());
        assert!(posture_with(Vec::new(), Vec::new(), None).is_ok());
        assert_eq!(
            posture_with(Vec::new(), Vec::new(), Some(0o1000)),
            Err(RoleContractError::InvalidUmask)
        );
    }

    #[test]
    fn posture_list_bounds_and_mount_paths_are_refused() {
        let capabilities = (0..=MAX_ROLE_POSTURE_CAPABILITIES)
            .map(|index| BoundedToken::parse(format!("cap-{index}")).unwrap())
            .collect::<Vec<_>>();
        assert!(
            posture_with(
                capabilities[..MAX_ROLE_POSTURE_CAPABILITIES].to_vec(),
                Vec::new(),
                None,
            )
            .is_ok()
        );
        assert_eq!(
            posture_with(capabilities, Vec::new(), None),
            Err(RoleContractError::BoundExceeded)
        );
        let mounts = (0..=MAX_ROLE_POSTURE_MOUNTS)
            .map(|index| {
                RoleMount::new(
                    RoleMountPath::parse(format!("/mnt/{index}")).unwrap(),
                    false,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            posture_with(Vec::new(), mounts, None),
            Err(RoleContractError::BoundExceeded)
        );
        for value in [
            "var/lib/d2b".to_owned(),
            "/with\u{7f}control".to_owned(),
            format!("/{}", "p".repeat(MAX_PATH_BYTES)),
        ] {
            assert_eq!(
                RoleMountPath::parse(value),
                Err(RoleContractError::InvalidMountPath)
            );
        }
    }

    #[test]
    fn roles_with_posture_round_trip_through_canonical_json() {
        let json = concat!(
            r#"{"rules":[{"resourceTypes":["Process"],"verbs":["get"],"subresources":[],"resourceNames":["worker"],"zones":[],"executionRefs":[],"sessionVerbs":[]}],"#,
            r#""operationRefs":["Operation/start"],"commandRefs":["Command/worker"],"#,
            r#""posture":{"seccompRef":"SeccompProfile/worker","principalRef":"Principal/worker","capabilities":["cap-net-bind"],"namespaces":{"mount":true,"pid":false,"net":true,"uts":false,"ipc":false,"cgroup":false,"time":false},"mounts":[{"path":"/var/lib/d2b","writable":true}],"umask":18,"userNs":true}}"#,
        );
        let role: RoleSpec = serde_json::from_str(json).unwrap();
        let posture = role.posture().unwrap();
        assert_eq!(posture.seccomp_ref().name().as_str(), "worker");
        assert_eq!(posture.principal_ref().name(), "worker");
        assert_eq!(posture.capabilities()[0].as_str(), "cap-net-bind");
        assert!(posture.namespaces().net());
        assert!(!posture.namespaces().ipc());
        assert_eq!(posture.mounts()[0].path().as_str(), "/var/lib/d2b");
        assert!(posture.mounts()[0].writable());
        assert_eq!(posture.umask(), Some(0o022));
        assert!(posture.user_ns());
        assert_eq!(serde_json::to_string(&role).unwrap(), json);
        assert_eq!(
            CanonicalJsonObject::parse(&serde_json::to_vec(&role).unwrap())
                .unwrap()
                .to_canonical_bytes(),
            CanonicalJsonObject::parse(json.as_bytes())
                .unwrap()
                .to_canonical_bytes()
        );
    }

    #[test]
    fn role_facets_default_to_the_empty_facet() {
        let role: RoleSpec = serde_json::from_str(
            r#"{"rules":[{"resourceTypes":["Process"],"verbs":["get"],"subresources":[],"resourceNames":["worker"],"zones":[],"executionRefs":[],"sessionVerbs":[]}]}"#,
        )
        .unwrap();
        assert!(role.operation_refs().is_empty());
        assert!(role.command_refs().is_empty());
        assert!(role.posture().is_none());
        let role: RoleSpec = serde_json::from_str(
            r#"{"rules":[{"resourceTypes":["Process"],"verbs":["get"],"subresources":[],"resourceNames":["worker"],"zones":[],"executionRefs":[],"sessionVerbs":[]}],"posture":{"seccompRef":"SeccompProfile/worker","principalRef":"Principal/worker"}}"#,
        )
        .unwrap();
        let posture = role.posture().unwrap();
        assert!(posture.capabilities().is_empty());
        assert!(posture.mounts().is_empty());
        assert_eq!(posture.umask(), None);
        assert!(!posture.user_ns());
        assert!(!posture.namespaces().mount());
    }
}
