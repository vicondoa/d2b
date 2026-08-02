//! Process and EphemeralProcess primitive ResourceType base specs.
//!
//! `ExecutionSpec` carries the frozen common field names shared by both
//! ResourceTypes. Those names are never renamed to `network`, `devices`, a
//! command, binary, or argv field, or an endpoint kind, path, or service
//! field, and there is no inline `endpoints` field: a stable endpoint a
//! Process produces is a separate owned `Endpoint` resource.
//!
//! No free-form executable, raw host path, numeric UID or GID, raw seccomp
//! program, ambient capability list, caller-selected broker operation,
//! credential bytes, or arbitrary socket address is accepted. `template` is a
//! plain bounded ID resolved by the owning semantic Provider, not a
//! ResourceRef.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        BoundedToken, BudgetSpec, DurationMs, ExecutionDomain, PrimitiveSpecError, redacted_debug,
        require_execution_ref, require_resource_type,
    },
};

/// The canonical ResourceType name for the long-lived process type.
pub const PROCESS_RESOURCE_TYPE: &str = "Process";
/// The canonical ResourceType name for the one-shot process type.
pub const EPHEMERAL_PROCESS_RESOURCE_TYPE: &str = "EphemeralProcess";
/// Maximum Credential references on one process.
pub const MAX_CREDENTIAL_REFS: usize = 16;
/// Maximum Volume mounts on one process.
pub const MAX_MOUNTS: usize = 64;
/// Maximum device usages on one process.
pub const MAX_DEVICE_USAGES: usize = 16;
/// Maximum declared inbound ports on one process.
pub const MAX_PORTS: usize = 256;
/// Maximum namespace classes on one sandbox.
pub const MAX_NAMESPACE_CLASSES: usize = 8;
/// Maximum capability classes on one sandbox.
pub const MAX_CAPABILITY_CLASSES: usize = 16;
/// Maximum bytes in an absolute mount path.
pub const MAX_MOUNT_PATH_BYTES: usize = 255;

/// Process classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ProcessClass {
    Controller,
    Service,
    Worker,
}

/// Semantic namespace isolation request.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum NamespaceClass {
    User,
    Pid,
    Mount,
    Ipc,
    Uts,
    Network,
    Cgroup,
    Time,
}

/// Semantic capability grant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityClass {
    NetworkBind,
    NetworkRaw,
    NetworkAdmin,
    SysTime,
    SysPtrace,
    SysAdmin,
    DacOverride,
    Fowner,
    Chown,
    Setuid,
    Setgid,
    AuditWrite,
    Kill,
}

/// Semantic environment inheritance class.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentClass {
    Minimal,
    SafeInherited,
    ProviderDefined,
}

/// Frozen semantic user-namespace mapping class.
///
/// No numeric host UID or GID appears in the public spec; core resolves the
/// exact identifiers into private launch state only.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MappingClass {
    ProcessPrincipalRoot,
}

/// The single-entry user namespace pre-established before exec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserNamespaceSpec {
    pub mapping_class: MappingClass,
}

/// Semantic sandbox requirements compiled by the selected Process Provider.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpec {
    namespace_classes: Vec<NamespaceClass>,
    capability_classes: Vec<CapabilityClass>,
    seccomp_class: BoundedToken,
    no_new_privileges: bool,
    start_root: bool,
    environment_class: EnvironmentClass,
    read_only_root: bool,
    umask: Option<String>,
    oom_score_adj: i32,
    user_namespace: Option<UserNamespaceSpec>,
}

impl SandboxSpec {
    /// Construct a sandbox specification after checking every frozen bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace_classes: Vec<NamespaceClass>,
        capability_classes: Vec<CapabilityClass>,
        seccomp_class: BoundedToken,
        no_new_privileges: bool,
        start_root: bool,
        environment_class: EnvironmentClass,
        read_only_root: bool,
        umask: Option<String>,
        oom_score_adj: i32,
        user_namespace: Option<UserNamespaceSpec>,
    ) -> Result<Self, PrimitiveSpecError> {
        check_unique(&namespace_classes, MAX_NAMESPACE_CLASSES)?;
        check_unique(&capability_classes, MAX_CAPABILITY_CLASSES)?;
        if !no_new_privileges && !start_root {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        if let Some(umask) = &umask {
            validate_octal_mode(umask, 3, 4)?;
        }
        if !(-1000..=1000).contains(&oom_score_adj) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            namespace_classes,
            capability_classes,
            seccomp_class,
            no_new_privileges,
            start_root,
            environment_class,
            read_only_root,
            umask,
            oom_score_adj,
            user_namespace,
        })
    }

    /// Borrow the requested namespace classes.
    pub fn namespace_classes(&self) -> &[NamespaceClass] {
        &self.namespace_classes
    }

    /// Borrow the requested capability classes.
    pub fn capability_classes(&self) -> &[CapabilityClass] {
        &self.capability_classes
    }

    /// Borrow the seccomp policy class.
    pub const fn seccomp_class(&self) -> &BoundedToken {
        &self.seccomp_class
    }

    /// Whether `PR_SET_NO_NEW_PRIVS` is installed before exec.
    pub const fn no_new_privileges(&self) -> bool {
        self.no_new_privileges
    }

    /// Whether the process starts as the in-namespace root UID.
    pub const fn start_root(&self) -> bool {
        self.start_root
    }

    /// Return the environment inheritance class.
    pub const fn environment_class(&self) -> EnvironmentClass {
        self.environment_class
    }

    /// Whether the root filesystem is mounted read-only.
    pub const fn read_only_root(&self) -> bool {
        self.read_only_root
    }

    /// Return the OOM score adjustment.
    pub const fn oom_score_adj(&self) -> i32 {
        self.oom_score_adj
    }

    /// Borrow the user-namespace request.
    pub const fn user_namespace(&self) -> Option<&UserNamespaceSpec> {
        self.user_namespace.as_ref()
    }
}

impl Default for SandboxSpec {
    fn default() -> Self {
        Self {
            namespace_classes: Vec::new(),
            capability_classes: Vec::new(),
            seccomp_class: BoundedToken::parse("strict").expect("strict is a valid token"),
            no_new_privileges: true,
            start_root: false,
            environment_class: EnvironmentClass::Minimal,
            read_only_root: true,
            umask: Some("0022".to_owned()),
            oom_score_adj: 0,
            user_namespace: None,
        }
    }
}

redacted_debug!(SandboxSpec);

impl<'de> Deserialize<'de> for SandboxSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            namespace_classes: Vec<NamespaceClass>,
            #[serde(default)]
            capability_classes: Vec<CapabilityClass>,
            #[serde(default)]
            seccomp_class: Option<BoundedToken>,
            #[serde(default = "yes")]
            no_new_privileges: bool,
            #[serde(default)]
            start_root: bool,
            #[serde(default = "minimal_environment")]
            environment_class: EnvironmentClass,
            #[serde(default = "yes")]
            read_only_root: bool,
            #[serde(default = "default_umask")]
            umask: Option<String>,
            #[serde(default)]
            oom_score_adj: i32,
            #[serde(default)]
            user_namespace: Option<UserNamespaceSpec>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let seccomp_class = match wire.seccomp_class {
            Some(class) => class,
            None => BoundedToken::parse("strict").map_err(serde::de::Error::custom)?,
        };
        Self::new(
            wire.namespace_classes,
            wire.capability_classes,
            seccomp_class,
            wire.no_new_privileges,
            wire.start_root,
            wire.environment_class,
            wire.read_only_root,
            wire.umask,
            wire.oom_score_adj,
            wire.user_namespace,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Access level of one Volume mount.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum MountAccess {
    ReadOnly,
    ReadWrite,
}

/// One Volume mount exposed inside the process sandbox.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MountSpec {
    volume_ref: ResourceRef,
    view: BoundedToken,
    mount_path: String,
    access: MountAccess,
    required: bool,
}

impl MountSpec {
    /// Construct a mount after checking the reference type and mount path.
    pub fn new(
        volume_ref: ResourceRef,
        view: BoundedToken,
        mount_path: impl Into<String>,
        access: MountAccess,
        required: bool,
    ) -> Result<Self, PrimitiveSpecError> {
        require_resource_type(&volume_ref, "Volume")?;
        let mount_path = mount_path.into();
        validate_absolute_path(&mount_path)?;
        Ok(Self {
            volume_ref,
            view,
            mount_path,
            access,
            required,
        })
    }

    /// Borrow the mounted Volume.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// Borrow the selected Volume view.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Borrow the sandbox-side mount path.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }

    /// Return the mount access level.
    pub const fn access(&self) -> MountAccess {
        self.access
    }

    /// Whether process start fails when the Volume is not Ready.
    pub const fn required(&self) -> bool {
        self.required
    }
}

redacted_debug!(MountSpec);

impl<'de> Deserialize<'de> for MountSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            volume_ref: ResourceRef,
            view: BoundedToken,
            mount_path: String,
            #[serde(default = "read_only")]
            access: MountAccess,
            #[serde(default = "yes")]
            required: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.volume_ref,
            wire.view,
            wire.mount_path,
            wire.access,
            wire.required,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Transport protocol of one declared port.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
    Udp,
    Sctp,
}

/// One declared inbound port.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PortSpec {
    port: u16,
    protocol: PortProtocol,
    purpose: String,
}

impl PortSpec {
    /// Construct a port declaration after checking its bounds.
    pub fn new(
        port: u16,
        protocol: PortProtocol,
        purpose: impl Into<String>,
    ) -> Result<Self, PrimitiveSpecError> {
        if port == 0 {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        let purpose = purpose.into();
        validate_purpose(&purpose)?;
        Ok(Self {
            port,
            protocol,
            purpose,
        })
    }

    /// Return the port number.
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Return the transport protocol.
    pub const fn protocol(&self) -> PortProtocol {
        self.protocol
    }

    /// Borrow the stable service label.
    pub fn purpose(&self) -> &str {
        &self.purpose
    }
}

redacted_debug!(PortSpec);

impl<'de> Deserialize<'de> for PortSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            port: u16,
            #[serde(default = "tcp")]
            protocol: PortProtocol,
            #[serde(default)]
            purpose: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.port, wire.protocol, wire.purpose).map_err(serde::de::Error::custom)
    }
}

/// Network access declared by one process.
///
/// The field is named `networkUsage` on the execution spec and is never
/// renamed to `network`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkUsageSpec {
    network_ref: Option<ResourceRef>,
    ports: Vec<PortSpec>,
    allow_egress: bool,
}

impl NetworkUsageSpec {
    /// Construct a network usage after checking bounds and the reference type.
    pub fn new(
        network_ref: Option<ResourceRef>,
        ports: Vec<PortSpec>,
        allow_egress: bool,
    ) -> Result<Self, PrimitiveSpecError> {
        if let Some(network_ref) = &network_ref {
            require_resource_type(network_ref, "Network")?;
        }
        if ports.len() > MAX_PORTS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            network_ref,
            ports,
            allow_egress,
        })
    }

    /// Borrow the named Network.
    pub const fn network_ref(&self) -> Option<&ResourceRef> {
        self.network_ref.as_ref()
    }

    /// Borrow the declared inbound ports.
    pub fn ports(&self) -> &[PortSpec] {
        &self.ports
    }

    /// Whether the process may initiate outbound connections.
    pub const fn allow_egress(&self) -> bool {
        self.allow_egress
    }
}

redacted_debug!(NetworkUsageSpec);

impl<'de> Deserialize<'de> for NetworkUsageSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            network_ref: Option<ResourceRef>,
            #[serde(default)]
            ports: Vec<PortSpec>,
            #[serde(default)]
            allow_egress: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.network_ref, wire.ports, wire.allow_egress).map_err(serde::de::Error::custom)
    }
}

/// Device access level requested by one process.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DeviceAccess {
    Shared,
    Exclusive,
}

/// One device access declared by a process.
///
/// The field is named `deviceUsage` on the execution spec and is never
/// renamed to `devices`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceUsageSpec {
    device_ref: ResourceRef,
    access: DeviceAccess,
    purpose: String,
}

impl DeviceUsageSpec {
    /// Construct a device usage after checking the reference type.
    pub fn new(
        device_ref: ResourceRef,
        access: DeviceAccess,
        purpose: impl Into<String>,
    ) -> Result<Self, PrimitiveSpecError> {
        require_resource_type(&device_ref, "Device")?;
        let purpose = purpose.into();
        validate_purpose(&purpose)?;
        Ok(Self {
            device_ref,
            access,
            purpose,
        })
    }

    /// Borrow the referenced Device.
    pub const fn device_ref(&self) -> &ResourceRef {
        &self.device_ref
    }

    /// Return the requested access level.
    pub const fn access(&self) -> DeviceAccess {
        self.access
    }

    /// Borrow the bounded usage purpose.
    pub fn purpose(&self) -> &str {
        &self.purpose
    }
}

redacted_debug!(DeviceUsageSpec);

impl<'de> Deserialize<'de> for DeviceUsageSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            device_ref: ResourceRef,
            #[serde(default = "shared")]
            access: DeviceAccess,
            #[serde(default)]
            purpose: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.device_ref, wire.access, wire.purpose).map_err(serde::de::Error::custom)
    }
}

/// Log level hint carried by the telemetry bindings.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
}

/// Telemetry and observability bindings.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TelemetrySpec {
    #[serde(default = "yes")]
    pub metrics_enabled: bool,
    #[serde(default = "yes")]
    pub tracing_enabled: bool,
    #[serde(default = "info_level")]
    pub log_level: LogLevel,
    #[serde(default)]
    pub sensitive_labels: bool,
}

impl Default for TelemetrySpec {
    fn default() -> Self {
        Self {
            metrics_enabled: true,
            tracing_enabled: true,
            log_level: LogLevel::Info,
            sensitive_labels: false,
        }
    }
}

redacted_debug!(TelemetrySpec);

/// The execution fields shared by Process and EphemeralProcess.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSpec {
    execution_ref: ResourceRef,
    domain: Option<ExecutionDomain>,
    user_ref: Option<ResourceRef>,
    process_class: ProcessClass,
    template: BoundedToken,
    config_ref: Option<ResourceRef>,
    credential_refs: Vec<ResourceRef>,
    mounts: Vec<MountSpec>,
    sandbox: SandboxSpec,
    budget: BudgetSpec,
    network_usage: Option<NetworkUsageSpec>,
    device_usage: Vec<DeviceUsageSpec>,
    telemetry: TelemetrySpec,
}

impl ExecutionSpec {
    /// Construct the shared execution fields after checking every bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        execution_ref: ResourceRef,
        domain: Option<ExecutionDomain>,
        user_ref: Option<ResourceRef>,
        process_class: ProcessClass,
        template: BoundedToken,
        config_ref: Option<ResourceRef>,
        credential_refs: Vec<ResourceRef>,
        mounts: Vec<MountSpec>,
        sandbox: SandboxSpec,
        budget: BudgetSpec,
        network_usage: Option<NetworkUsageSpec>,
        device_usage: Vec<DeviceUsageSpec>,
        telemetry: TelemetrySpec,
    ) -> Result<Self, PrimitiveSpecError> {
        require_execution_ref(&execution_ref)?;
        if let Some(user_ref) = &user_ref {
            require_resource_type(user_ref, "User")?;
        }
        if let Some(config_ref) = &config_ref {
            require_resource_type(config_ref, "Volume")?;
        }
        if credential_refs.len() > MAX_CREDENTIAL_REFS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        for credential_ref in &credential_refs {
            require_resource_type(credential_ref, "Credential")?;
        }
        if mounts.len() > MAX_MOUNTS || device_usage.len() > MAX_DEVICE_USAGES {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        let mount_paths: BTreeSet<_> = mounts.iter().map(MountSpec::mount_path).collect();
        if mount_paths.len() != mounts.len() {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        let device_refs: BTreeSet<_> = device_usage
            .iter()
            .map(DeviceUsageSpec::device_ref)
            .collect();
        if device_refs.len() != device_usage.len() {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        if domain == Some(ExecutionDomain::User) && sandbox.start_root() {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self {
            execution_ref,
            domain,
            user_ref,
            process_class,
            template,
            config_ref,
            credential_refs,
            mounts,
            sandbox,
            budget,
            network_usage,
            device_usage,
            telemetry,
        })
    }

    /// Construct the canonical minimal execution fields.
    pub fn minimal(
        execution_ref: ResourceRef,
        process_class: ProcessClass,
        template: BoundedToken,
    ) -> Result<Self, PrimitiveSpecError> {
        Self::new(
            execution_ref,
            None,
            None,
            process_class,
            template,
            None,
            Vec::new(),
            Vec::new(),
            SandboxSpec::default(),
            BudgetSpec::default(),
            None,
            Vec::new(),
            TelemetrySpec::default(),
        )
    }

    /// Borrow the target Host or Guest.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Return the explicit execution domain.
    pub const fn domain(&self) -> Option<ExecutionDomain> {
        self.domain
    }

    /// Borrow the explicit user identity.
    pub const fn user_ref(&self) -> Option<&ResourceRef> {
        self.user_ref.as_ref()
    }

    /// Return the process classification.
    pub const fn process_class(&self) -> ProcessClass {
        self.process_class
    }

    /// Borrow the plain component template ID.
    pub const fn template(&self) -> &BoundedToken {
        &self.template
    }

    /// Borrow the sealed config Volume.
    pub const fn config_ref(&self) -> Option<&ResourceRef> {
        self.config_ref.as_ref()
    }

    /// Borrow the sealed Credential references.
    pub fn credential_refs(&self) -> &[ResourceRef] {
        &self.credential_refs
    }

    /// Borrow the declared Volume mounts.
    pub fn mounts(&self) -> &[MountSpec] {
        &self.mounts
    }

    /// Borrow the semantic sandbox requirements.
    pub const fn sandbox(&self) -> &SandboxSpec {
        &self.sandbox
    }

    /// Borrow the per-process budget.
    pub const fn budget(&self) -> &BudgetSpec {
        &self.budget
    }

    /// Borrow the network usage.
    pub const fn network_usage(&self) -> Option<&NetworkUsageSpec> {
        self.network_usage.as_ref()
    }

    /// Borrow the device usages.
    pub fn device_usage(&self) -> &[DeviceUsageSpec] {
        &self.device_usage
    }

    /// Return the telemetry bindings.
    pub const fn telemetry(&self) -> TelemetrySpec {
        self.telemetry
    }
}

redacted_debug!(ExecutionSpec);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecutionWire {
    execution_ref: ResourceRef,
    #[serde(default)]
    domain: Option<ExecutionDomain>,
    #[serde(default)]
    user_ref: Option<ResourceRef>,
    process_class: ProcessClass,
    template: BoundedToken,
    #[serde(default)]
    config_ref: Option<ResourceRef>,
    #[serde(default)]
    credential_refs: Vec<ResourceRef>,
    #[serde(default)]
    mounts: Vec<MountSpec>,
    #[serde(default)]
    sandbox: SandboxSpec,
    #[serde(default)]
    budget: BudgetSpec,
    #[serde(default)]
    network_usage: Option<NetworkUsageSpec>,
    #[serde(default)]
    device_usage: Vec<DeviceUsageSpec>,
    #[serde(default)]
    telemetry: TelemetrySpec,
}

impl ExecutionWire {
    fn into_execution(self) -> Result<ExecutionSpec, PrimitiveSpecError> {
        ExecutionSpec::new(
            self.execution_ref,
            self.domain,
            self.user_ref,
            self.process_class,
            self.template,
            self.config_ref,
            self.credential_refs,
            self.mounts,
            self.sandbox,
            self.budget,
            self.network_usage,
            self.device_usage,
            self.telemetry,
        )
    }
}

impl<'de> Deserialize<'de> for ExecutionSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ExecutionWire::deserialize(deserializer)?
            .into_execution()
            .map_err(serde::de::Error::custom)
    }
}

/// Desired steady-state lifecycle of a Process.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DesiredLifecycle {
    Running,
    Stopped,
}

/// Restart classification.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum RestartClass {
    Never,
    Always,
    OnFailure,
    OnCrash,
}

/// Restart and backoff behavior.
///
/// The backoff multiplier is integer fixed-point because canonical JSON
/// admits no floating-point number.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestartPolicySpec {
    class: RestartClass,
    backoff_base: DurationMs,
    backoff_max: DurationMs,
    backoff_multiplier_milli: u32,
    max_restarts: Option<u32>,
    reset_after: DurationMs,
}

impl RestartPolicySpec {
    /// Construct a restart policy after checking every frozen bound.
    pub fn new(
        class: RestartClass,
        backoff_base: DurationMs,
        backoff_max: DurationMs,
        backoff_multiplier_milli: u32,
        max_restarts: Option<u32>,
        reset_after: DurationMs,
    ) -> Result<Self, PrimitiveSpecError> {
        check_duration(&backoff_base, 0, 60_000)?;
        check_duration(&backoff_max, 1_000, 3_600_000)?;
        check_duration(&reset_after, 0, 86_400_000)?;
        if !(1_000..=10_000).contains(&backoff_multiplier_milli) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        if let Some(max_restarts) = max_restarts
            && !(1..=65_535).contains(&max_restarts)
        {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        Ok(Self {
            class,
            backoff_base,
            backoff_max,
            backoff_multiplier_milli,
            max_restarts,
            reset_after,
        })
    }

    /// Return the restart classification.
    pub const fn class(&self) -> RestartClass {
        self.class
    }

    /// Return the integer fixed-point backoff multiplier.
    pub const fn backoff_multiplier_milli(&self) -> u32 {
        self.backoff_multiplier_milli
    }

    /// Return the restart ceiling.
    pub const fn max_restarts(&self) -> Option<u32> {
        self.max_restarts
    }
}

impl Default for RestartPolicySpec {
    fn default() -> Self {
        Self {
            class: RestartClass::OnFailure,
            backoff_base: duration("1s", 0, 60_000),
            backoff_max: duration("60s", 1_000, 3_600_000),
            backoff_multiplier_milli: 2_000,
            max_restarts: None,
            reset_after: duration("300s", 0, 86_400_000),
        }
    }
}

redacted_debug!(RestartPolicySpec);

impl<'de> Deserialize<'de> for RestartPolicySpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "on_failure")]
            class: RestartClass,
            #[serde(default)]
            backoff_base: Option<DurationMs>,
            #[serde(default)]
            backoff_max: Option<DurationMs>,
            #[serde(default = "two_thousand")]
            backoff_multiplier_milli: u32,
            #[serde(default)]
            max_restarts: Option<u32>,
            #[serde(default)]
            reset_after: Option<DurationMs>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let default = RestartPolicySpec::default();
        Self::new(
            wire.class,
            wire.backoff_base.unwrap_or(default.backoff_base),
            wire.backoff_max.unwrap_or(default.backoff_max),
            wire.backoff_multiplier_milli,
            wire.max_restarts,
            wire.reset_after.unwrap_or(default.reset_after),
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Readiness probe mechanism.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ReadinessClass {
    ReadyCondition,
    ProviderDefined,
}

/// Readiness probe settings.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessSpec {
    initial_delay: DurationMs,
    timeout: DurationMs,
    failure_threshold: u32,
    success_threshold: u32,
    class: ReadinessClass,
}

impl ReadinessSpec {
    /// Construct readiness settings after checking every frozen bound.
    pub fn new(
        initial_delay: DurationMs,
        timeout: DurationMs,
        failure_threshold: u32,
        success_threshold: u32,
        class: ReadinessClass,
    ) -> Result<Self, PrimitiveSpecError> {
        check_duration(&initial_delay, 0, 300_000)?;
        check_duration(&timeout, 1_000, 300_000)?;
        check_threshold(failure_threshold)?;
        check_threshold(success_threshold)?;
        Ok(Self {
            initial_delay,
            timeout,
            failure_threshold,
            success_threshold,
            class,
        })
    }

    /// Return the readiness mechanism.
    pub const fn class(&self) -> ReadinessClass {
        self.class
    }
}

impl Default for ReadinessSpec {
    fn default() -> Self {
        Self {
            initial_delay: duration("0s", 0, 300_000),
            timeout: duration("30s", 1_000, 300_000),
            failure_threshold: 3,
            success_threshold: 1,
            class: ReadinessClass::ReadyCondition,
        }
    }
}

redacted_debug!(ReadinessSpec);

impl<'de> Deserialize<'de> for ReadinessSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            initial_delay: Option<DurationMs>,
            #[serde(default)]
            timeout: Option<DurationMs>,
            #[serde(default = "three")]
            failure_threshold: u32,
            #[serde(default = "one")]
            success_threshold: u32,
            #[serde(default = "ready_condition")]
            class: ReadinessClass,
        }
        let wire = Wire::deserialize(deserializer)?;
        let default = ReadinessSpec::default();
        Self::new(
            wire.initial_delay.unwrap_or(default.initial_delay),
            wire.timeout.unwrap_or(default.timeout),
            wire.failure_threshold,
            wire.success_threshold,
            wire.class,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Health check mechanism.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum HealthCheckClass {
    ProviderDefined,
}

/// Ongoing health check settings.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckSpec {
    enabled: bool,
    interval: DurationMs,
    timeout: DurationMs,
    failure_threshold: u32,
    class: HealthCheckClass,
}

impl HealthCheckSpec {
    /// Construct health check settings after checking every frozen bound.
    pub fn new(
        enabled: bool,
        interval: DurationMs,
        timeout: DurationMs,
        failure_threshold: u32,
        class: HealthCheckClass,
    ) -> Result<Self, PrimitiveSpecError> {
        check_duration(&interval, 1_000, 3_600_000)?;
        check_duration(&timeout, 1_000, 60_000)?;
        check_threshold(failure_threshold)?;
        Ok(Self {
            enabled,
            interval,
            timeout,
            failure_threshold,
            class,
        })
    }

    /// Whether ongoing health checks run after readiness.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
}

impl Default for HealthCheckSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: duration("30s", 1_000, 3_600_000),
            timeout: duration("5s", 1_000, 60_000),
            failure_threshold: 3,
            class: HealthCheckClass::ProviderDefined,
        }
    }
}

redacted_debug!(HealthCheckSpec);

impl<'de> Deserialize<'de> for HealthCheckSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            enabled: bool,
            #[serde(default)]
            interval: Option<DurationMs>,
            #[serde(default)]
            timeout: Option<DurationMs>,
            #[serde(default = "three")]
            failure_threshold: u32,
            #[serde(default = "provider_defined_health")]
            class: HealthCheckClass,
        }
        let wire = Wire::deserialize(deserializer)?;
        let default = HealthCheckSpec::default();
        Self::new(
            wire.enabled,
            wire.interval.unwrap_or(default.interval),
            wire.timeout.unwrap_or(default.timeout),
            wire.failure_threshold,
            wire.class,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Whether the controller adopts a running process after restart.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AdoptionPolicy {
    AdoptOnRestart,
    NeverAdopt,
}

/// The Process ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSpec {
    #[serde(flatten)]
    execution: ExecutionSpec,
    desired_lifecycle: DesiredLifecycle,
    restart_policy: RestartPolicySpec,
    readiness: ReadinessSpec,
    health_check: HealthCheckSpec,
    adoption_policy: AdoptionPolicy,
    drain_timeout: DurationMs,
}

impl ProcessSpec {
    /// Construct a Process base spec after checking every frozen bound.
    pub fn new(
        execution: ExecutionSpec,
        desired_lifecycle: DesiredLifecycle,
        restart_policy: RestartPolicySpec,
        readiness: ReadinessSpec,
        health_check: HealthCheckSpec,
        adoption_policy: AdoptionPolicy,
        drain_timeout: DurationMs,
    ) -> Result<Self, PrimitiveSpecError> {
        check_duration(&drain_timeout, 0, 3_600_000)?;
        Ok(Self {
            execution,
            desired_lifecycle,
            restart_policy,
            readiness,
            health_check,
            adoption_policy,
            drain_timeout,
        })
    }

    /// Construct the canonical minimal Process base spec.
    pub fn minimal(execution: ExecutionSpec) -> Self {
        Self {
            execution,
            desired_lifecycle: DesiredLifecycle::Running,
            restart_policy: RestartPolicySpec::default(),
            readiness: ReadinessSpec::default(),
            health_check: HealthCheckSpec::default(),
            adoption_policy: AdoptionPolicy::AdoptOnRestart,
            drain_timeout: duration("30s", 0, 3_600_000),
        }
    }

    /// Borrow the shared execution fields.
    pub const fn execution(&self) -> &ExecutionSpec {
        &self.execution
    }

    /// Return the desired steady-state lifecycle.
    pub const fn desired_lifecycle(&self) -> DesiredLifecycle {
        self.desired_lifecycle
    }

    /// Borrow the restart policy.
    pub const fn restart_policy(&self) -> &RestartPolicySpec {
        &self.restart_policy
    }

    /// Borrow the readiness settings.
    pub const fn readiness(&self) -> &ReadinessSpec {
        &self.readiness
    }

    /// Borrow the health check settings.
    pub const fn health_check(&self) -> &HealthCheckSpec {
        &self.health_check
    }

    /// Return the adoption policy.
    pub const fn adoption_policy(&self) -> AdoptionPolicy {
        self.adoption_policy
    }
}

redacted_debug!(ProcessSpec);

impl<'de> Deserialize<'de> for ProcessSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            execution_ref: ResourceRef,
            #[serde(default)]
            domain: Option<ExecutionDomain>,
            #[serde(default)]
            user_ref: Option<ResourceRef>,
            process_class: ProcessClass,
            template: BoundedToken,
            #[serde(default)]
            config_ref: Option<ResourceRef>,
            #[serde(default)]
            credential_refs: Vec<ResourceRef>,
            #[serde(default)]
            mounts: Vec<MountSpec>,
            #[serde(default)]
            sandbox: SandboxSpec,
            #[serde(default)]
            budget: BudgetSpec,
            #[serde(default)]
            network_usage: Option<NetworkUsageSpec>,
            #[serde(default)]
            device_usage: Vec<DeviceUsageSpec>,
            #[serde(default)]
            telemetry: TelemetrySpec,
            #[serde(default = "running")]
            desired_lifecycle: DesiredLifecycle,
            #[serde(default)]
            restart_policy: RestartPolicySpec,
            #[serde(default)]
            readiness: ReadinessSpec,
            #[serde(default)]
            health_check: HealthCheckSpec,
            #[serde(default = "adopt_on_restart")]
            adoption_policy: AdoptionPolicy,
            #[serde(default)]
            drain_timeout: Option<DurationMs>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let execution = ExecutionWire {
            execution_ref: wire.execution_ref,
            domain: wire.domain,
            user_ref: wire.user_ref,
            process_class: wire.process_class,
            template: wire.template,
            config_ref: wire.config_ref,
            credential_refs: wire.credential_refs,
            mounts: wire.mounts,
            sandbox: wire.sandbox,
            budget: wire.budget,
            network_usage: wire.network_usage,
            device_usage: wire.device_usage,
            telemetry: wire.telemetry,
        }
        .into_execution()
        .map_err(serde::de::Error::custom)?;
        Self::new(
            execution,
            wire.desired_lifecycle,
            wire.restart_policy,
            wire.readiness,
            wire.health_check,
            wire.adoption_policy,
            wire.drain_timeout
                .unwrap_or_else(|| duration("30s", 0, 3_600_000)),
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The EphemeralProcess ResourceType base spec.
///
/// EphemeralProcess is the one-shot process itself, never a job that
/// references a Process, and it carries no restart, readiness, health check,
/// or adoption field.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EphemeralProcessSpec {
    #[serde(flatten)]
    execution: ExecutionSpec,
    start_deadline: DurationMs,
    runtime_deadline: DurationMs,
    successful_ttl: DurationMs,
    failed_ttl: DurationMs,
    incident_hold: bool,
}

impl EphemeralProcessSpec {
    /// Construct an EphemeralProcess base spec after checking every bound.
    pub fn new(
        execution: ExecutionSpec,
        start_deadline: DurationMs,
        runtime_deadline: DurationMs,
        successful_ttl: DurationMs,
        failed_ttl: DurationMs,
        incident_hold: bool,
    ) -> Result<Self, PrimitiveSpecError> {
        if execution.process_class() != ProcessClass::Worker {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        check_duration(&start_deadline, 1_000, 3_600_000)?;
        check_duration(&runtime_deadline, 1_000, 86_400_000)?;
        check_duration(&successful_ttl, 0, 7 * 86_400_000)?;
        check_duration(&failed_ttl, 0, 30 * 86_400_000)?;
        Ok(Self {
            execution,
            start_deadline,
            runtime_deadline,
            successful_ttl,
            failed_ttl,
            incident_hold,
        })
    }

    /// Construct the canonical minimal EphemeralProcess base spec.
    pub fn minimal(execution: ExecutionSpec) -> Result<Self, PrimitiveSpecError> {
        Self::new(
            execution,
            duration("60s", 1_000, 3_600_000),
            duration("300s", 1_000, 86_400_000),
            duration("1h", 0, 7 * 86_400_000),
            duration("24h", 0, 30 * 86_400_000),
            false,
        )
    }

    /// Borrow the shared execution fields.
    pub const fn execution(&self) -> &ExecutionSpec {
        &self.execution
    }

    /// Borrow the successful terminal retention.
    pub const fn successful_ttl(&self) -> &DurationMs {
        &self.successful_ttl
    }

    /// Borrow the failed terminal retention.
    pub const fn failed_ttl(&self) -> &DurationMs {
        &self.failed_ttl
    }

    /// Whether cleanup is blocked pending an explicit release.
    pub const fn incident_hold(&self) -> bool {
        self.incident_hold
    }
}

redacted_debug!(EphemeralProcessSpec);

impl<'de> Deserialize<'de> for EphemeralProcessSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            execution_ref: ResourceRef,
            #[serde(default)]
            domain: Option<ExecutionDomain>,
            #[serde(default)]
            user_ref: Option<ResourceRef>,
            process_class: ProcessClass,
            template: BoundedToken,
            #[serde(default)]
            config_ref: Option<ResourceRef>,
            #[serde(default)]
            credential_refs: Vec<ResourceRef>,
            #[serde(default)]
            mounts: Vec<MountSpec>,
            #[serde(default)]
            sandbox: SandboxSpec,
            #[serde(default)]
            budget: BudgetSpec,
            #[serde(default)]
            network_usage: Option<NetworkUsageSpec>,
            #[serde(default)]
            device_usage: Vec<DeviceUsageSpec>,
            #[serde(default)]
            telemetry: TelemetrySpec,
            #[serde(default)]
            start_deadline: Option<DurationMs>,
            #[serde(default)]
            runtime_deadline: Option<DurationMs>,
            #[serde(default)]
            successful_ttl: Option<DurationMs>,
            #[serde(default)]
            failed_ttl: Option<DurationMs>,
            #[serde(default)]
            incident_hold: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        let execution = ExecutionWire {
            execution_ref: wire.execution_ref,
            domain: wire.domain,
            user_ref: wire.user_ref,
            process_class: wire.process_class,
            template: wire.template,
            config_ref: wire.config_ref,
            credential_refs: wire.credential_refs,
            mounts: wire.mounts,
            sandbox: wire.sandbox,
            budget: wire.budget,
            network_usage: wire.network_usage,
            device_usage: wire.device_usage,
            telemetry: wire.telemetry,
        }
        .into_execution()
        .map_err(serde::de::Error::custom)?;
        Self::new(
            execution,
            wire.start_deadline
                .unwrap_or_else(|| duration("60s", 1_000, 3_600_000)),
            wire.runtime_deadline
                .unwrap_or_else(|| duration("300s", 1_000, 86_400_000)),
            wire.successful_ttl
                .unwrap_or_else(|| duration("1h", 0, 7 * 86_400_000)),
            wire.failed_ttl
                .unwrap_or_else(|| duration("24h", 0, 30 * 86_400_000)),
            wire.incident_hold,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn check_unique<T: Ord + Clone>(values: &[T], max: usize) -> Result<(), PrimitiveSpecError> {
    if values.len() > max {
        return Err(PrimitiveSpecError::TooManyEntries);
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted.len() == values.len() {
        Ok(())
    } else {
        Err(PrimitiveSpecError::DuplicateEntry)
    }
}

fn check_duration(value: &DurationMs, min: u64, max: u64) -> Result<(), PrimitiveSpecError> {
    if value.as_millis() < min || value.as_millis() > max {
        Err(PrimitiveSpecError::InvalidDuration)
    } else {
        Ok(())
    }
}

fn check_threshold(value: u32) -> Result<(), PrimitiveSpecError> {
    if (1..=100).contains(&value) {
        Ok(())
    } else {
        Err(PrimitiveSpecError::OutOfRange)
    }
}

fn validate_purpose(value: &str) -> Result<(), PrimitiveSpecError> {
    if value.len() > 63 || value.chars().any(char::is_control) {
        Err(PrimitiveSpecError::InvalidText)
    } else {
        Ok(())
    }
}

fn validate_absolute_path(value: &str) -> Result<(), PrimitiveSpecError> {
    if !value.starts_with('/')
        || value.len() > MAX_MOUNT_PATH_BYTES
        || value.split('/').any(|segment| segment == "..")
        || value.contains('\0')
    {
        Err(PrimitiveSpecError::InvalidPath)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_octal_mode(
    value: &str,
    min_digits: usize,
    max_digits: usize,
) -> Result<(), PrimitiveSpecError> {
    if (min_digits..=max_digits).contains(&value.len())
        && value.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
    {
        Ok(())
    } else {
        Err(PrimitiveSpecError::InvalidMode)
    }
}

fn duration(text: &str, min: u64, max: u64) -> DurationMs {
    DurationMs::parse(text, min, max).expect("frozen default durations are always valid")
}

const fn yes() -> bool {
    true
}

const fn one() -> u32 {
    1
}

const fn three() -> u32 {
    3
}

const fn two_thousand() -> u32 {
    2_000
}

const fn minimal_environment() -> EnvironmentClass {
    EnvironmentClass::Minimal
}

fn default_umask() -> Option<String> {
    Some("0022".to_owned())
}

const fn read_only() -> MountAccess {
    MountAccess::ReadOnly
}

const fn tcp() -> PortProtocol {
    PortProtocol::Tcp
}

const fn shared() -> DeviceAccess {
    DeviceAccess::Shared
}

const fn info_level() -> LogLevel {
    LogLevel::Info
}

const fn on_failure() -> RestartClass {
    RestartClass::OnFailure
}

const fn ready_condition() -> ReadinessClass {
    ReadinessClass::ReadyCondition
}

const fn provider_defined_health() -> HealthCheckClass {
    HealthCheckClass::ProviderDefined
}

const fn running() -> DesiredLifecycle {
    DesiredLifecycle::Running
}

const fn adopt_on_restart() -> AdoptionPolicy {
    AdoptionPolicy::AdoptOnRestart
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    fn minimal_execution() -> ExecutionSpec {
        ExecutionSpec::minimal(
            ResourceRef::parse("Host/host-system").unwrap(),
            ProcessClass::Controller,
            BoundedToken::parse("controller-main").unwrap(),
        )
        .unwrap()
    }

    const MINIMAL_PROCESS_SPEC: &[u8] = br#"{"adoptionPolicy":"adopt-on-restart","budget":{},"configRef":null,"credentialRefs":[],"desiredLifecycle":"running","deviceUsage":[],"domain":null,"drainTimeout":"30s","executionRef":"Host/host-system","healthCheck":{"class":"provider-defined","enabled":false,"failureThreshold":3,"interval":"30s","timeout":"5s"},"mounts":[],"networkUsage":null,"processClass":"controller","readiness":{"class":"ready-condition","failureThreshold":3,"initialDelay":"0s","successThreshold":1,"timeout":"30s"},"restartPolicy":{"backoffBase":"1s","backoffMax":"60s","backoffMultiplierMilli":2000,"class":"on-failure","maxRestarts":null,"resetAfter":"300s"},"sandbox":{"capabilityClasses":[],"environmentClass":"minimal","namespaceClasses":[],"noNewPrivileges":true,"oomScoreAdj":0,"readOnlyRoot":true,"seccompClass":"strict","startRoot":false,"umask":"0022","userNamespace":null},"telemetry":{"logLevel":"info","metricsEnabled":true,"sensitiveLabels":false,"tracingEnabled":true},"template":"controller-main","userRef":null}"#;

    #[test]
    fn schema_vector_pins_the_minimal_process_base_spec() {
        let spec = ProcessSpec::minimal(minimal_execution());
        assert_eq!(canonical_json_bytes(&spec).unwrap(), MINIMAL_PROCESS_SPEC);
        let parsed: ProcessSpec = serde_json::from_slice(MINIMAL_PROCESS_SPEC).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn the_frozen_common_field_names_are_never_renamed() {
        let base = to_base_object(&ProcessSpec::minimal(minimal_execution())).unwrap();
        for present in ["networkUsage", "deviceUsage", "template", "executionRef"] {
            assert!(base.get(present).is_some());
        }
        for absent in [
            "network",
            "devices",
            "endpoints",
            "command",
            "binary",
            "argv",
            "providerRef",
            "updatePolicy",
            "provider",
        ] {
            assert!(base.get(absent).is_none());
        }
    }

    #[test]
    fn renamed_or_free_form_execution_fields_are_rejected() {
        for rejected in [
            br#"{"executionRef":"Host/h","processClass":"worker","template":"t","network":null}"#.as_slice(),
            br#"{"executionRef":"Host/h","processClass":"worker","template":"t","devices":[]}"#,
            br#"{"executionRef":"Host/h","processClass":"worker","template":"t","command":"/bin/sh"}"#,
            br#"{"executionRef":"Host/h","processClass":"worker","template":"t","argv":[]}"#,
            br#"{"executionRef":"Host/h","processClass":"worker","template":"t","endpoints":[]}"#,
        ] {
            assert!(serde_json::from_slice::<ProcessSpec>(rejected).is_err());
        }
    }

    #[test]
    fn execution_ref_and_folded_refs_are_type_checked() {
        assert_eq!(
            ExecutionSpec::minimal(
                ResourceRef::parse("Provider/system-core").unwrap(),
                ProcessClass::Worker,
                BoundedToken::parse("t").unwrap(),
            ),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        assert!(
            ExecutionSpec::minimal(
                ResourceRef::parse("Guest/dev-vm").unwrap(),
                ProcessClass::Worker,
                BoundedToken::parse("t").unwrap(),
            )
            .is_ok()
        );
        assert_eq!(
            MountSpec::new(
                ResourceRef::parse("Host/h").unwrap(),
                BoundedToken::parse("controller").unwrap(),
                "/state",
                MountAccess::ReadWrite,
                true,
            ),
            Err(PrimitiveSpecError::WrongResourceType)
        );
        assert_eq!(
            MountSpec::new(
                ResourceRef::parse("Volume/state").unwrap(),
                BoundedToken::parse("controller").unwrap(),
                "state",
                MountAccess::ReadWrite,
                true,
            ),
            Err(PrimitiveSpecError::InvalidPath)
        );
    }

    #[test]
    fn direct_execution_spec_rejects_unknown_fields_and_duplicate_mounts() {
        let unknown = br#"{"executionRef":"Host/host-system","processClass":"worker","template":"t","unknown":true}"#;
        assert!(serde_json::from_slice::<ExecutionSpec>(unknown).is_err());

        let first = MountSpec::new(
            ResourceRef::parse("Volume/state").unwrap(),
            BoundedToken::parse("state").unwrap(),
            "/state",
            MountAccess::ReadOnly,
            true,
        )
        .unwrap();
        let second = MountSpec::new(
            ResourceRef::parse("Volume/other").unwrap(),
            BoundedToken::parse("state").unwrap(),
            "/state",
            MountAccess::ReadOnly,
            true,
        )
        .unwrap();
        assert_eq!(
            ExecutionSpec::new(
                ResourceRef::parse("Host/host-system").unwrap(),
                None,
                None,
                ProcessClass::Worker,
                BoundedToken::parse("t").unwrap(),
                None,
                Vec::new(),
                vec![first, second],
                SandboxSpec::default(),
                BudgetSpec::default(),
                None,
                Vec::new(),
                TelemetrySpec::default(),
            ),
            Err(PrimitiveSpecError::DuplicateEntry)
        );
    }

    #[test]
    fn ephemeral_process_is_worker_only_and_carries_no_restart_fields() {
        let controller = ExecutionSpec::minimal(
            ResourceRef::parse("Host/host-system").unwrap(),
            ProcessClass::Controller,
            BoundedToken::parse("t").unwrap(),
        )
        .unwrap();
        assert_eq!(
            EphemeralProcessSpec::minimal(controller),
            Err(PrimitiveSpecError::ConflictingFields)
        );

        let worker = ExecutionSpec::minimal(
            ResourceRef::parse("Host/host-system").unwrap(),
            ProcessClass::Worker,
            BoundedToken::parse("swtpm-flush").unwrap(),
        )
        .unwrap();
        let spec = EphemeralProcessSpec::minimal(worker).unwrap();
        assert_eq!(spec.successful_ttl().as_str(), "1h");
        assert_eq!(spec.failed_ttl().as_str(), "24h");
        let base = to_base_object(&spec).unwrap();
        for absent in [
            "restartPolicy",
            "readiness",
            "healthCheck",
            "adoptionPolicy",
        ] {
            assert!(base.get(absent).is_none());
        }
    }

    #[test]
    fn the_backoff_multiplier_is_integer_fixed_point() {
        let base = to_base_object(&RestartPolicySpec::default()).unwrap();
        assert!(base.get("backoffMultiplier").is_none());
        assert!(base.get("backoffMultiplierMilli").is_some());
        assert!(
            serde_json::from_slice::<RestartPolicySpec>(br#"{"backoffMultiplier":2.0}"#).is_err()
        );
        assert!(
            serde_json::from_slice::<RestartPolicySpec>(br#"{"backoffMultiplierMilli":500}"#)
                .is_err()
        );
    }

    #[test]
    fn sandbox_rejects_numeric_and_raw_implementation_fields() {
        for rejected in [
            br#"{"uidMap":"0 1000 1"}"#.as_slice(),
            br#"{"seccompFilter":[]}"#,
            br#"{"capabilities":["CAP_SYS_ADMIN"]}"#,
        ] {
            assert!(serde_json::from_slice::<SandboxSpec>(rejected).is_err());
        }
        assert_eq!(
            SandboxSpec::new(
                Vec::new(),
                Vec::new(),
                BoundedToken::parse("strict").unwrap(),
                false,
                false,
                EnvironmentClass::Minimal,
                true,
                None,
                0,
                None,
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert!(
            serde_json::from_slice::<SandboxSpec>(
                br#"{"userNamespace":{"mappingClass":"process-principal-root"}}"#
            )
            .is_ok()
        );
    }

    #[test]
    fn diagnostics_stay_redacted() {
        let spec = ProcessSpec::minimal(minimal_execution());
        assert_eq!(format!("{spec:?}"), "ProcessSpec(<redacted>)");
        assert_eq!(
            format!("{:?}", spec.execution()),
            "ExecutionSpec(<redacted>)"
        );
    }
}
