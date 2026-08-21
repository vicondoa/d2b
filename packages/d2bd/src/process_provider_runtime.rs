//! Daemon-owned composition of the fixed process Providers.
//!
//! The Provider crates remain pure controllers: they receive only the
//! core-owned effect ports. This module is the one production seam that
//! constructs those ports from the authenticated broker transport and the
//! trusted bundle. No Provider receives a broker socket or a bundle resolver.

use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_resource::v3::{
    execution_policy::{BoundedToken, ExecutionDomain},
};
use d2b_contracts_resource::v3::{
    ControllerGeneration,
    ResourceGeneration,
    ResourceRef,
    ResourceUid,
    process::{EphemeralProcessSpec, ProcessSpec},
};
use d2b_core::{
    bundle_resolver::BundleResolver,
    processes::{ProcessNode, ProcessRole},
};
use d2b_process_conformance::{
    AdoptionOutcome, CompiledDigests, ConfigurationDigest, IdentityBinding, LaunchTicket,
    OperationBinding, ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort,
    ProcessProvider, ProcessStatusReport, ReadinessExpectation, SandboxCompiler, StopClass,
};
use d2b_provider_supervisor::{
    BrokerProcessBackend, BrokerSystemdEffectOwner, BundleBackedLaunchResolver, ProviderSupervisor,
    SystemdProcessBackend,
};
use d2b_provider_system_minijail::{MinijailProcessProvider, launch::PlatformGate};
use d2b_provider_system_systemd::SystemdProcessProvider;
use sha2::{Digest, Sha256};

/// The fixed process Provider names wired by the daemon.
pub const FIXED_PROCESS_PROVIDER_NAMES: [&str; 2] = ["system-minijail", "system-systemd"];

type BrokerProcessSupervisor = ProviderSupervisor<BrokerProcessBackend<BundleBackedLaunchResolver>>;
type BrokerSystemdSupervisor = ProviderSupervisor<SystemdProcessBackend<BrokerSystemdEffectOwner>>;

/// Probe the host posture needed by the daemon-owned minijail Provider.
///
/// The Provider receives this bounded snapshot through its constructor; it
/// never reads host paths or cgroup state itself.
pub(crate) fn detect_minijail_platform_gate() -> PlatformGate {
    let (kernel_major, kernel_minor) = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .and_then(|release| {
            let mut components = release.split('.');
            let major = components.next()?.parse().ok()?;
            let minor = components
                .next()
                .and_then(|component| {
                    component
                        .split(|character: char| !character.is_ascii_digit())
                        .next()
                })
                .filter(|component| !component.is_empty())?
                .parse()
                .ok()?;
            Some((major, minor))
        })
        .unwrap_or((0, 0));
    let cgroup_kill_writable = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|cgroup| {
            let relative = cgroup
                .lines()
                .find_map(|line| line.strip_prefix("0::"))?
                .trim()
                .trim_start_matches('/')
                .to_owned();
            let path = std::path::Path::new("/sys/fs/cgroup")
                .join(relative)
                .join("cgroup.kill");
            path.is_file().then_some(path)
        })
        .is_some_and(|path| OpenOptions::new().write(true).open(path).is_ok());
    PlatformGate::from_observed(kernel_major, kernel_minor, cgroup_kill_writable)
}

fn retryable_stop_error(error: &str) -> bool {
    matches!(
        error,
        "stop-failed"
            | "observe-failed"
            | "launch-failed"
            | "effect-adapter-busy"
            | "deadline-exceeded"
            | "process-fate-unknown"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProvider {
    Minijail,
    Systemd,
}

#[derive(Debug, Clone, Copy)]
struct ManagedProcess {
    provider: ManagedProvider,
    identity: ProcessIdentityDigest,
}

#[derive(Debug, Clone)]
struct ManagedResource {
    provider: ManagedProvider,
    identity: ProcessIdentityDigest,
    uid: ResourceUid,
    generation: ResourceGeneration,
}

fn resource_identity_matches(
    managed: &ManagedResource,
    context: ProcessResourceContext<'_>,
) -> bool {
    managed.uid == *context.resource_uid && managed.generation == context.resource_generation
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessResourceContext<'a> {
    pub(crate) resource_ref: &'a ResourceRef,
    pub(crate) resource_uid: &'a ResourceUid,
    pub(crate) resource_generation: ResourceGeneration,
    pub(crate) provider_ref: &'a ResourceRef,
    pub(crate) controller_generation: ControllerGeneration,
}

impl<'a> ProcessResourceContext<'a> {
    pub(crate) const fn new(
        resource_ref: &'a ResourceRef,
        resource_uid: &'a ResourceUid,
        resource_generation: ResourceGeneration,
        provider_ref: &'a ResourceRef,
        controller_generation: ControllerGeneration,
    ) -> Self {
        Self {
            resource_ref,
            resource_uid,
            resource_generation,
            provider_ref,
            controller_generation,
        }
    }
}

/// Result of a Provider-backed launch, carrying only opaque process identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderLaunch {
    /// Opaque identity established by the effect adapter.
    pub identity: ProcessIdentityDigest,
}

/// Result of a Provider-backed adoption attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAdoption {
    /// No process matching the trusted ticket is running.
    Absent,
    /// The exact process was adopted.
    Adopted(ProcessStatusReport),
    /// A candidate was present but identity was ambiguous and quarantined.
    Quarantined(ProcessStatusReport),
}

/// Provider-backed liveness result used by the daemon readiness loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLiveness {
    /// The exact process is still present.
    Alive,
    /// The exact process is absent.
    Exited,
    /// Identity could not be established safely.
    Unknown,
}

/// Readiness-loop adapter for one Provider-managed process node.
pub struct ProviderLivenessProbe {
    providers: Arc<ProductionProcessProviders>,
    vm: String,
    node: ProcessNode,
}

impl ProviderLivenessProbe {
    /// Bind a Provider composition to one immutable process-DAG node.
    pub fn new(
        providers: Arc<ProductionProcessProviders>,
        vm: impl Into<String>,
        node: &ProcessNode,
    ) -> Self {
        Self {
            providers,
            vm: vm.into(),
            node: node.clone(),
        }
    }
}

impl d2bd_runtime::supervisor::readiness_liveness::LivenessProbe for ProviderLivenessProbe {
    fn probe(&self) -> d2bd_runtime::supervisor::readiness_liveness::RunnerLiveness {
        match crate::block_on_future(self.providers.probe_node(&self.vm, &self.node)) {
            Ok(ProviderLiveness::Alive) => {
                d2bd_runtime::supervisor::readiness_liveness::RunnerLiveness::Alive
            }
            Ok(ProviderLiveness::Exited) => {
                d2bd_runtime::supervisor::readiness_liveness::RunnerLiveness::Exited(None)
            }
            Ok(ProviderLiveness::Unknown) | Err(_) => {
                d2bd_runtime::supervisor::readiness_liveness::RunnerLiveness::Unknown
            }
        }
    }
}

/// Production process Provider controllers.
///
/// The concrete supervisors are retained by the daemon for its whole
/// lifetime. Their internal handles and broker effect owners never cross the
/// Provider boundary; Provider code sees only the
/// `ProcessLaunchEffectPort` implemented by `ProviderSupervisor`.
pub struct ProductionProcessProviders {
    minijail: MinijailProcessProvider<BrokerProcessSupervisor>,
    systemd: SystemdProcessProvider<BrokerSystemdSupervisor>,
    bundle: BundleResolver,
    managed: Mutex<BTreeMap<(String, String), ManagedProcess>>,
    managed_resources: Mutex<BTreeMap<ResourceRef, ManagedResource>>,
}

impl std::fmt::Debug for ProductionProcessProviders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionProcessProviders")
            .field("providers", &FIXED_PROCESS_PROVIDER_NAMES)
            .finish()
    }
}

impl ProductionProcessProviders {
    /// Construct both fixed process Providers over the authenticated broker.
    pub fn new(
        bundle: BundleResolver,
        broker_socket: impl Into<PathBuf>,
        caller_role: BrokerCallerRole,
    ) -> Self {
        let broker_socket = broker_socket.into();
        let resolver = BundleBackedLaunchResolver::new(bundle.clone()).with_observation_socket(
            broker_socket.clone(),
            Duration::from_secs(10),
            caller_role.clone(),
        );
        let minijail_backend = BrokerProcessBackend::with_socket_and_role(
            resolver.clone(),
            broker_socket.clone(),
            Duration::from_secs(10),
            caller_role.clone(),
        );
        let systemd_owner = BrokerSystemdEffectOwner::with_socket(
            resolver,
            broker_socket,
            Duration::from_secs(10),
            caller_role,
        );
        let platform_gate = detect_minijail_platform_gate();
        Self {
            minijail: MinijailProcessProvider::with_platform_gate(
                ProviderSupervisor::new(minijail_backend),
                platform_gate,
            ),
            systemd: SystemdProcessProvider::new(ProviderSupervisor::new(
                SystemdProcessBackend::new(systemd_owner),
            )),
            bundle,
            managed: Mutex::new(BTreeMap::new()),
            managed_resources: Mutex::new(BTreeMap::new()),
        }
    }

    /// Borrow the daemon-owned minijail Provider.
    pub const fn minijail(&self) -> &MinijailProcessProvider<BrokerProcessSupervisor> {
        &self.minijail
    }

    /// Borrow the daemon-owned systemd Provider.
    pub const fn systemd(&self) -> &SystemdProcessProvider<BrokerSystemdSupervisor> {
        &self.systemd
    }

    /// Return the fixed Provider names in contract order.
    pub const fn provider_names() -> &'static [&'static str; 2] {
        &FIXED_PROCESS_PROVIDER_NAMES
    }

    /// Return whether this node is a daemon-owned Provider process.
    pub fn supports_node(node: &ProcessNode) -> bool {
        matches!(
            node.role,
            ProcessRole::SwtpmPreStartFlush
                | ProcessRole::Swtpm
                | ProcessRole::Virtiofsd
                | ProcessRole::CloudHypervisorRunner
                | ProcessRole::QemuMediaRunner
                | ProcessRole::Gpu
                | ProcessRole::GpuRenderNode
                | ProcessRole::Audio
                | ProcessRole::Video
                | ProcessRole::VsockRelay
                | ProcessRole::OtelHostBridge
                | ProcessRole::Usbip
                | ProcessRole::WaylandProxy
        )
    }

    /// Return whether this node remains supervised after its start step.
    pub fn is_long_lived(node: &ProcessNode) -> bool {
        !matches!(node.role, ProcessRole::SwtpmPreStartFlush) && Self::supports_node(node)
    }

    /// Return the stable role key used by the broker and daemon stop paths.
    pub fn tracked_role_id(node: &ProcessNode) -> String {
        if matches!(node.role, ProcessRole::CloudHypervisorRunner) {
            "ch-runner".to_owned()
        } else {
            node.id.0.clone()
        }
    }

    /// Return all Provider-managed long-lived roles declared for one VM.
    pub fn managed_role_ids(&self, vm: &str) -> Vec<String> {
        let Some(dag) = self.bundle.find_process_vm(vm) else {
            return Vec::new();
        };
        dag.nodes
            .iter()
            .filter(|node| Self::is_long_lived(node))
            .map(Self::tracked_role_id)
            .collect()
    }

    /// Return a cloned trusted process node for a tracked role key.
    pub fn node_for_role(&self, vm: &str, role_id: &str) -> Option<ProcessNode> {
        self.bundle
            .find_process_vm(vm)?
            .nodes
            .iter()
            .find(|node| Self::tracked_role_id(node) == role_id)
            .cloned()
    }

    /// Return every VM that has a process DAG in the trusted bundle.
    pub fn vm_ids(&self) -> Vec<String> {
        self.bundle
            .processes
            .vms
            .iter()
            .map(|dag| dag.vm.clone())
            .collect()
    }

    /// Return whether a Provider-managed identity is currently retained.
    pub fn has_active_role(&self, vm: &str, role_id: &str) -> bool {
        self.managed
            .lock()
            .map(|managed| managed.contains_key(&(vm.to_owned(), role_id.to_owned())))
            .unwrap_or(false)
    }

    /// Return whether any Provider-managed long-lived role is retained.
    pub fn has_active_vm(&self, vm: &str) -> bool {
        self.managed
            .lock()
            .map(|managed| managed.keys().any(|(managed_vm, _)| managed_vm == vm))
            .unwrap_or(false)
    }

    /// Return Provider role keys with retained exact local authority.
    pub fn active_role_ids(&self, vm: &str) -> Vec<String> {
        self.managed
            .lock()
            .map(|managed| {
                managed
                    .keys()
                    .filter(|(managed_vm, _)| managed_vm == vm)
                    .map(|(_, role)| role.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Launch one trusted process node through its selected fixed Provider.
    pub async fn launch_node(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        let ticket = self.ticket_with_timeout(vm, node, timeout)?;
        let provider = self.provider_for(node);
        let report = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .launch(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => {
                self.systemd.launch(&ticket).await.map_err(provider_error)?
            }
        };
        self.remember(vm, node, report.identity)?;
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    /// Launch one durable Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn launch_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        let provider = managed_provider_from_ref(context.provider_ref)?;
        let ticket = resource_ticket(
            &self.bundle,
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
            provider,
            timeout,
        )?;
        let report = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .launch(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => {
                self.systemd.launch(&ticket).await.map_err(provider_error)?
            }
        };
        self.remember_resource(
            context.resource_ref,
            context.resource_uid,
            context.resource_generation,
            provider,
            report.identity,
        )?;
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    /// Launch one ephemeral Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn launch_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        let provider = managed_provider_from_ref(context.provider_ref)?;
        let ticket = resource_ticket(
            &self.bundle,
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
            provider,
            timeout,
        )?;
        let report = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .launch(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => {
                self.systemd.launch(&ticket).await.map_err(provider_error)?
            }
        };
        self.remember_resource(
            context.resource_ref,
            context.resource_uid,
            context.resource_generation,
            provider,
            report.identity,
        )?;
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    /// Adopt one durable Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn adopt_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.adopt_resource_with_execution(
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
        )
        .await
    }

    async fn stop_resource_identity_with_retry(
        &self,
        managed: &ManagedResource,
        class: StopClass,
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            match self.stop_resource_identity(managed, class).await {
                Ok(()) => return Ok(()),
                Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {
                    return Err(error);
                }
                Err(error) if retryable_stop_error(&error) && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Adopt one ephemeral Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn adopt_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.adopt_resource_with_execution(
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
        )
        .await
    }

    /// Probe one durable Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn probe_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.probe_resource_with_execution(
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
        )
        .await
    }

    /// Probe one ephemeral Process resource with the controller generation
    /// rehydrated from the owning Zone store.
    pub(crate) async fn probe_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.probe_resource_with_execution(
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
        )
        .await
    }

    /// Stop one exact generic Process identity with the controller
    /// generation rehydrated from the owning Zone store.
    pub(crate) async fn stop_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.stop_resource_with_execution(
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
            term_timeout,
            kill_timeout,
        )
        .await
    }

    /// Stop one exact generic EphemeralProcess identity with the controller
    /// generation rehydrated from the owning Zone store.
    pub(crate) async fn stop_ephemeral_resource(
        &self,
        context: ProcessResourceContext<'_>,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.stop_resource_with_execution(
            context,
            spec.execution(),
            &serde_json::to_vec(spec).map_err(|_| "provider-ticket:serialization".to_owned())?,
            term_timeout,
            kill_timeout,
        )
        .await
    }

    /// Finalize one terminal generic Process identity.
    pub(crate) async fn finalize_resource(
        &self,
        context: ProcessResourceContext<'_>,
    ) -> Result<(), String> {
        let Some(managed) = self
            .managed_resources
            .lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .get(context.resource_ref)
            .cloned()
        else {
            return Ok(());
        };
        if !resource_identity_matches(&managed, context) {
            return Err("provider-process-identity-changed".to_owned());
        }
        let result = match managed.provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
        };
        match result {
            Ok(()) => {
                self.forget_resource(context.resource_ref);
                Ok(())
            }
            Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {
                self.forget_resource(context.resource_ref);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Return whether a generic resource retains a verified identity.
    pub fn has_active_resource(&self, resource_ref: &ResourceRef) -> bool {
        self.managed_resources
            .lock()
            .map(|managed| managed.contains_key(resource_ref))
            .unwrap_or(false)
    }

    async fn adopt_resource_with_execution(
        &self,
        context: ProcessResourceContext<'_>,
        execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
        spec_bytes: &[u8],
    ) -> Result<ProviderAdoption, String> {
        let provider = managed_provider_from_ref(context.provider_ref)?;
        let ticket = resource_ticket(
            &self.bundle,
            context,
            execution,
            spec_bytes,
            provider,
            Duration::from_secs(30),
        )?;
        let outcome = match provider {
            ManagedProvider::Minijail => {
                self.minijail.adopt(&ticket).await.map_err(provider_error)?
            }
            ManagedProvider::Systemd => {
                self.systemd.adopt(&ticket).await.map_err(provider_error)?
            }
        };
        match outcome {
            AdoptionOutcome::Absent => Ok(ProviderAdoption::Absent),
            AdoptionOutcome::Adopted(report) => {
                self.remember_resource(
                    context.resource_ref,
                    context.resource_uid,
                    context.resource_generation,
                    provider,
                    report.identity,
                )?;
                Ok(ProviderAdoption::Adopted(report))
            }
            AdoptionOutcome::Quarantined(report) => {
                self.forget_resource(context.resource_ref);
                Ok(ProviderAdoption::Quarantined(report))
            }
        }
    }

    async fn probe_resource_with_execution(
        &self,
        context: ProcessResourceContext<'_>,
        execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
        spec_bytes: &[u8],
    ) -> Result<ProviderLiveness, String> {
        let provider = managed_provider_from_ref(context.provider_ref)?;
        let ticket = resource_ticket(
            &self.bundle,
            context,
            execution,
            spec_bytes,
            provider,
            Duration::from_secs(30),
        )?;
        let candidate = match provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
        };
        let Some(candidate) = candidate else {
            return Ok(ProviderLiveness::Exited);
        };
        let (expected_owner, required) = match provider {
            ManagedProvider::Minijail => (
                self.minijail.profile().wait_reap_owner(),
                self.minijail.profile().required_identity_bindings(),
            ),
            ManagedProvider::Systemd => (
                self.systemd.profile().wait_reap_owner(),
                self.systemd.profile().required_identity_bindings(),
            ),
        };
        if candidate.wait_reap_owner != expected_owner || candidate.validate(required).is_err() {
            Ok(ProviderLiveness::Unknown)
        } else {
            Ok(ProviderLiveness::Alive)
        }
    }

    async fn stop_resource_with_execution(
        &self,
        context: ProcessResourceContext<'_>,
        execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
        spec_bytes: &[u8],
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        let managed = self
            .managed_resources
            .lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .get(context.resource_ref)
            .cloned()
            .ok_or_else(|| "provider-process-not-found".to_owned())?;
        if managed.uid != *context.resource_uid || managed.generation != context.resource_generation
        {
            return Err("provider-process-identity-changed".to_owned());
        }
        match self
            .stop_resource_identity_with_retry(
                &managed,
                StopClass::Drain,
                Instant::now() + term_timeout,
            )
            .await
        {
            Ok(()) => {}
            Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {}
            Err(error) => return Err(error),
        }
        let deadline = Instant::now() + term_timeout;
        loop {
            match self
                .probe_resource_with_execution(context, execution, spec_bytes)
                .await?
            {
                ProviderLiveness::Exited => {
                    self.finalize_resource(context).await?;
                    return Ok(false);
                }
                ProviderLiveness::Alive => {}
                ProviderLiveness::Unknown if Instant::now() >= deadline => break,
                ProviderLiveness::Unknown => {}
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        match self
            .stop_resource_identity_with_retry(
                &managed,
                StopClass::Terminate,
                Instant::now() + kill_timeout,
            )
            .await
        {
            Ok(()) => {}
            Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {}
            Err(error) => return Err(error),
        }
        let kill_deadline = Instant::now() + kill_timeout;
        loop {
            match self
                .probe_resource_with_execution(context, execution, spec_bytes)
                .await?
            {
                ProviderLiveness::Exited => {
                    self.finalize_resource(context).await?;
                    return Ok(true);
                }
                ProviderLiveness::Alive | ProviderLiveness::Unknown => {}
            }
            if Instant::now() >= kill_deadline {
                return Err("provider-process-kill-timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Adopt one trusted process node after a daemon restart.
    pub async fn adopt_node(
        &self,
        vm: &str,
        node: &ProcessNode,
    ) -> Result<ProviderAdoption, String> {
        let ticket = self.ticket(vm, node)?;
        let outcome = match self.provider_for(node) {
            ManagedProvider::Minijail => {
                self.minijail.adopt(&ticket).await.map_err(provider_error)?
            }
            ManagedProvider::Systemd => {
                self.systemd.adopt(&ticket).await.map_err(provider_error)?
            }
        };
        match outcome {
            AdoptionOutcome::Absent => Ok(ProviderAdoption::Absent),
            AdoptionOutcome::Adopted(report) => {
                self.remember(vm, node, report.identity)?;
                Ok(ProviderAdoption::Adopted(report))
            }
            AdoptionOutcome::Quarantined(report) => {
                self.forget(vm, node);
                Ok(ProviderAdoption::Quarantined(report))
            }
        }
    }

    /// Probe one node through the Provider's authenticated read-only path.
    ///
    /// Unlike adoption, a liveness probe does not open a pidfd, retain a
    /// handle, or stage an observation for a later adoption call.
    pub async fn probe_node(
        &self,
        vm: &str,
        node: &ProcessNode,
    ) -> Result<ProviderLiveness, String> {
        if !Self::supports_node(node) {
            return Ok(ProviderLiveness::Unknown);
        }
        let ticket = self.ticket(vm, node)?;
        let candidate = match self.provider_for(node) {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .probe(&ticket)
                .await
                .map_err(provider_error)?,
        };
        let Some(candidate) = candidate else {
            return Ok(ProviderLiveness::Exited);
        };
        let expected_owner = match self.provider_for(node) {
            ManagedProvider::Minijail => self.minijail.profile().wait_reap_owner(),
            ManagedProvider::Systemd => self.systemd.profile().wait_reap_owner(),
        };
        let required = match self.provider_for(node) {
            ManagedProvider::Minijail => self.minijail.profile().required_identity_bindings(),
            ManagedProvider::Systemd => self.systemd.profile().required_identity_bindings(),
        };
        if candidate.wait_reap_owner != expected_owner || candidate.validate(required).is_err() {
            Ok(ProviderLiveness::Unknown)
        } else {
            Ok(ProviderLiveness::Alive)
        }
    }

    /// Wait for a one-shot Provider process to exit and finalize its handle.
    pub async fn wait_for_exit(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.adopt_node(vm, node).await? {
                ProviderAdoption::Absent => {
                    self.finalize_node(vm, node).await?;
                    return Ok(());
                }
                ProviderAdoption::Adopted(_) => {}
                ProviderAdoption::Quarantined(_) => {
                    return Err("provider-process-quarantined".to_owned());
                }
            }
            if Instant::now() >= deadline {
                return Err("provider-process-exit-timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Stop one exact Provider identity, escalating after the drain budget.
    pub async fn stop_node(
        &self,
        vm: &str,
        node: &ProcessNode,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        let key = (vm.to_owned(), Self::tracked_role_id(node));
        let managed = self
            .managed
            .lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .get(&key)
            .copied()
            .ok_or_else(|| "provider-process-not-found".to_owned())?;
        match self.stop_identity(managed, StopClass::Drain).await {
            Ok(()) => {}
            Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {}
            Err(error) => return Err(error),
        }
        let deadline = Instant::now() + term_timeout;
        loop {
            match self.probe_node(vm, node).await? {
                ProviderLiveness::Exited => {
                    self.finalize_node(vm, node).await?;
                    return Ok(false);
                }
                ProviderLiveness::Alive => {}
                ProviderLiveness::Unknown => {
                    if Instant::now() >= deadline {
                        break;
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        match self.stop_identity(managed, StopClass::Terminate).await {
            Ok(()) => {}
            Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {}
            Err(error) => return Err(error),
        }
        let kill_deadline = Instant::now() + kill_timeout;
        loop {
            match self.probe_node(vm, node).await? {
                ProviderLiveness::Exited => {
                    self.finalize_node(vm, node).await?;
                    return Ok(true);
                }
                ProviderLiveness::Alive | ProviderLiveness::Unknown => {}
            }
            if Instant::now() >= kill_deadline {
                return Err("provider-process-kill-timeout".to_owned());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Finalize a terminal Provider process and remove its local authority.
    pub async fn finalize_node(&self, vm: &str, node: &ProcessNode) -> Result<(), String> {
        let key = (vm.to_owned(), Self::tracked_role_id(node));
        let Some(managed) = self
            .managed
            .lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .get(&key)
            .copied()
        else {
            return Ok(());
        };
        let result = match managed.provider {
            ManagedProvider::Minijail => self
                .minijail
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .port()
                .finalize_identity(&managed.identity)
                .await
                .map_err(provider_error),
        };
        match result {
            Ok(()) => {
                self.forget(vm, node);
                Ok(())
            }
            Err(error) if error == "pidfd-unavailable" || error == "process-vanished" => {
                self.forget(vm, node);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Adopt every long-lived process declared for one VM.
    pub async fn adopt_vm(&self, vm: &str) -> Result<(), String> {
        let Some(dag) = self.bundle.find_process_vm(vm) else {
            return Ok(());
        };
        for node in dag.nodes.iter().filter(|node| Self::is_long_lived(node)) {
            match self.adopt_node(vm, node).await? {
                ProviderAdoption::Absent => {
                    self.forget(vm, node);
                }
                ProviderAdoption::Adopted(_) => {}
                ProviderAdoption::Quarantined(_) => {
                    tracing::warn!(
                        vm = %vm,
                        role = %Self::tracked_role_id(node),
                        "Provider startup adoption quarantined an ambiguous process"
                    );
                }
            }
        }
        Ok(())
    }

    fn provider_for(&self, node: &ProcessNode) -> ManagedProvider {
        if node.unit.is_some() {
            ManagedProvider::Systemd
        } else {
            ManagedProvider::Minijail
        }
    }

    fn remember(
        &self,
        vm: &str,
        node: &ProcessNode,
        identity: ProcessIdentityDigest,
    ) -> Result<(), String> {
        self.managed
            .lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .insert(
                (vm.to_owned(), Self::tracked_role_id(node)),
                ManagedProcess {
                    provider: self.provider_for(node),
                    identity,
                },
            );
        Ok(())
    }

    fn remember_resource(
        &self,
        resource_ref: &ResourceRef,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        provider: ManagedProvider,
        identity: ProcessIdentityDigest,
    ) -> Result<(), String> {
        self.managed_resources
            .lock()
            .map_err(|_| "provider-managed-state-poisoned".to_owned())?
            .insert(
                resource_ref.clone(),
                ManagedResource {
                    provider,
                    identity,
                    uid: uid.clone(),
                    generation,
                },
            );
        Ok(())
    }

    fn forget(&self, vm: &str, node: &ProcessNode) {
        if let Ok(mut managed) = self.managed.lock() {
            managed.remove(&(vm.to_owned(), Self::tracked_role_id(node)));
        }
    }

    fn forget_resource(&self, resource_ref: &ResourceRef) {
        if let Ok(mut managed) = self.managed_resources.lock() {
            managed.remove(resource_ref);
        }
    }

    async fn stop_provider_identity(
        &self,
        provider: ManagedProvider,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), String> {
        match provider {
            ManagedProvider::Minijail => self
                .minijail
                .stop(identity, class)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .stop(identity, class)
                .await
                .map_err(provider_error),
        }
    }

    async fn stop_identity(&self, managed: ManagedProcess, class: StopClass) -> Result<(), String> {
        self.stop_provider_identity(managed.provider, &managed.identity, class)
            .await
    }

    async fn stop_resource_identity(
        &self,
        managed: &ManagedResource,
        class: StopClass,
    ) -> Result<(), String> {
        self.stop_provider_identity(managed.provider, &managed.identity, class)
            .await
    }

    fn ticket(&self, vm: &str, node: &ProcessNode) -> Result<LaunchTicket, String> {
        self.ticket_with_timeout(vm, node, Duration::from_secs(30))
    }

    fn ticket_with_timeout(
        &self,
        vm: &str,
        node: &ProcessNode,
        timeout: Duration,
    ) -> Result<LaunchTicket, String> {
        build_ticket(&self.bundle, vm, node, self.provider_for(node), timeout)
            .map_err(|error| format!("provider-ticket:{}", error.code()))
    }
}

fn provider_error(error: ProcessConformanceError) -> String {
    error.code().to_owned()
}

fn managed_provider_from_ref(provider_ref: &ResourceRef) -> Result<ManagedProvider, String> {
    match provider_ref.name().as_str() {
        "system-minijail" => Ok(ManagedProvider::Minijail),
        "system-systemd" => Ok(ManagedProvider::Systemd),
        _ => Err("provider-ticket:unsupported-provider".to_owned()),
    }
}

fn resource_ticket(
    bundle: &BundleResolver,
    context: ProcessResourceContext<'_>,
    execution: &d2b_contracts_resource::v3::process::ExecutionSpec,
    spec_bytes: &[u8],
    provider: ManagedProvider,
    timeout: Duration,
) -> Result<LaunchTicket, String> {
    let execution_domain = match execution.domain().unwrap_or(ExecutionDomain::System) {
        ExecutionDomain::System => d2b_core::processes::ProcessExecutionDomain::System,
        ExecutionDomain::User => d2b_core::processes::ProcessExecutionDomain::User,
    };
    let user_ref = execution.user_ref().map(ResourceRef::to_canonical_string);
    if bundle
        .find_runner_intent_for_process(
            &execution.execution_ref().to_canonical_string(),
            execution_domain,
            user_ref.as_deref(),
            execution.template().as_str(),
        )
        .is_none()
    {
        return Err("provider-ticket:template-not-found".to_owned());
    }
    let provider_name = context.provider_ref.name().as_str();
    let owner_provider =
        BoundedToken::parse(provider_name).map_err(|_| "provider-ticket:invalid-provider")?;
    let component = BoundedToken::parse("process-controller")
        .map_err(|_| "provider-ticket:invalid-component")?;
    let generation = context.resource_generation.get();
    let operation_uid = stable_uid(
        "operation",
        &context.resource_ref.to_canonical_string(),
        context.resource_uid.as_str(),
        generation,
    );
    let deadline_ms = timeout.as_millis().clamp(1, 900_000) as u32;
    let ticket = LaunchTicket::new(
        context.resource_ref.clone(),
        context.resource_uid.clone(),
        context.resource_generation,
        context.controller_generation,
        owner_provider.clone(),
        component,
        execution.template().clone(),
        execution.execution_ref().clone(),
        execution.domain().unwrap_or(ExecutionDomain::System),
        execution.user_ref().cloned(),
        owner_provider,
        compiled_resource_digests(bundle, context.resource_ref, provider, spec_bytes),
        OperationBinding::new(operation_uid, deadline_ms)
            .map_err(|_| "provider-ticket:invalid-operation")?,
        required_identity(provider),
    )
    .map_err(|error| format!("provider-ticket:{}", error.code()))?;
    let domain = execution.domain().unwrap_or(ExecutionDomain::System);
    let sandbox = if provider == ManagedProvider::Systemd {
        let spec = execution.sandbox();
        if !spec.namespace_classes().is_empty()
            || !spec.capability_classes().is_empty()
            || spec.seccomp_class().as_str() != "strict"
            || !spec.no_new_privileges()
            || spec.start_root()
            || !matches!(
                spec.environment_class(),
                d2b_contracts_resource::v3::process::EnvironmentClass::Minimal
            )
            || !spec.read_only_root()
            || spec.user_namespace().is_some()
        {
            return Err("provider-ticket:systemd-sandbox-unsupported".to_owned());
        }
        SandboxCompiler
            .compile_plan(spec, domain, false)
            .map_err(|error| format!("provider-ticket:{}", error.code()))?
    } else {
        SandboxCompiler
            .compile_plan(execution.sandbox(), domain, false)
            .map_err(|error| format!("provider-ticket:{}", error.code()))?
    };
    Ok(ticket
        .with_sandbox_plan(sandbox)
        .with_readiness(ReadinessExpectation::None))
}

fn compiled_resource_digests(
    bundle: &BundleResolver,
    resource_ref: &ResourceRef,
    provider: ManagedProvider,
    spec_bytes: &[u8],
) -> CompiledDigests {
    fn digest(label: &str, bytes: &[u8]) -> ConfigurationDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"d2bd-provider-resource-ticket-v1");
        hasher.update(label.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        ConfigurationDigest::from_bytes(hasher.finalize().into())
    }
    let context = format!(
        "{}:{}:{}",
        resource_ref.to_canonical_string(),
        match provider {
            ManagedProvider::Minijail => "system-minijail",
            ManagedProvider::Systemd => "system-systemd",
        },
        bundle.bundle.bundle_hash.as_deref().unwrap_or("bundle"),
    );
    CompiledDigests {
        sandbox: digest(&format!("{context}:sandbox"), spec_bytes),
        budget: digest(&format!("{context}:budget"), spec_bytes),
        mounts: digest(&format!("{context}:mounts"), spec_bytes),
        devices: digest(&format!("{context}:devices"), spec_bytes),
        network: digest(&format!("{context}:network"), spec_bytes),
        endpoints: digest(&format!("{context}:endpoints"), spec_bytes),
        fd_table: digest(&format!("{context}:fd-table"), spec_bytes),
    }
}

fn build_ticket(
    bundle: &BundleResolver,
    vm: &str,
    node: &ProcessNode,
    provider: ManagedProvider,
    timeout: Duration,
) -> Result<LaunchTicket, ProcessConformanceError> {
    let provider_name = match provider {
        ManagedProvider::Minijail => "system-minijail",
        ManagedProvider::Systemd => "system-systemd",
    };
    let process_type = if ProductionProcessProviders::is_long_lived(node) {
        "Process"
    } else {
        "EphemeralProcess"
    };
    let process_name = stable_token(&node.id.0);
    let process_ref = ResourceRef::parse(&format!("{process_type}/{process_name}"))
        .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let execution_ref = ResourceRef::parse(&format!("Guest/{vm}"))
        .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let owner_provider =
        BoundedToken::parse(provider_name).map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let component =
        BoundedToken::parse("vm-process").map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let template = BoundedToken::parse(stable_token(&node.id.0))
        .map_err(|_| ProcessConformanceError::InvalidTicket)?;
    let selected_provider = owner_provider.clone();
    let generation = stable_generation(bundle);
    let digests = compiled_digests(bundle, vm, node, provider);
    let operation_uid = stable_uid("operation", vm, &node.id.0, generation);
    let deadline_ms = timeout.as_millis().clamp(1, 900_000) as u32;
    let ticket = LaunchTicket::new(
        process_ref,
        stable_uid("process", vm, &node.id.0, generation),
        ResourceGeneration::new(generation).map_err(|_| ProcessConformanceError::InvalidTicket)?,
        ControllerGeneration::new(1).map_err(|_| ProcessConformanceError::InvalidTicket)?,
        owner_provider,
        component,
        template,
        execution_ref,
        ExecutionDomain::System,
        None,
        selected_provider,
        digests,
        OperationBinding::new(operation_uid, deadline_ms)?,
        required_identity(provider),
    )?;
    Ok(ticket.with_readiness(ReadinessExpectation::None))
}

fn required_identity(provider: ManagedProvider) -> std::collections::BTreeSet<IdentityBinding> {
    match provider {
        ManagedProvider::Minijail => std::collections::BTreeSet::from([
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
            IdentityBinding::Executable,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]),
        ManagedProvider::Systemd => std::collections::BTreeSet::from([
            IdentityBinding::UnitInvocationId,
            IdentityBinding::Cgroup,
            IdentityBinding::UnitMainPid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]),
    }
}

fn compiled_digests(
    bundle: &BundleResolver,
    vm: &str,
    node: &ProcessNode,
    provider: ManagedProvider,
) -> CompiledDigests {
    fn digest(label: &str, bytes: &[u8]) -> ConfigurationDigest {
        let mut hasher = Sha256::new();
        hasher.update(b"d2bd-provider-ticket-v1");
        hasher.update(label.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        ConfigurationDigest::from_bytes(hasher.finalize().into())
    }
    let node_bytes = serde_json::to_vec(node).unwrap_or_default();
    let context = format!(
        "{vm}:{}:{}:{}",
        node.id.0,
        match provider {
            ManagedProvider::Minijail => "system-minijail",
            ManagedProvider::Systemd => "system-systemd",
        },
        bundle.bundle.bundle_hash.as_deref().unwrap_or("bundle")
    );
    CompiledDigests {
        sandbox: digest(&format!("{context}:sandbox"), &node_bytes),
        budget: digest(&format!("{context}:budget"), &node_bytes),
        mounts: digest(&format!("{context}:mounts"), &node_bytes),
        devices: digest(&format!("{context}:devices"), &node_bytes),
        network: digest(&format!("{context}:network"), &node_bytes),
        endpoints: digest(&format!("{context}:endpoints"), &node_bytes),
        fd_table: digest(&format!("{context}:fd-table"), &node_bytes),
    }
}

fn stable_generation(bundle: &BundleResolver) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(
        bundle
            .bundle
            .bundle_hash
            .as_deref()
            .unwrap_or(bundle.bundle.generation.generator.as_str()),
    );
    let bytes: [u8; 32] = hasher.finalize().into();
    let generation = u64::from_le_bytes(bytes[..8].try_into().expect("digest prefix"));
    if generation == 0 { 1 } else { generation }
}

fn stable_uid(label: &str, vm: &str, role: &str, generation: u64) -> ResourceUid {
    let mut hasher = Sha256::new();
    hasher.update(b"d2bd-provider-resource-v1");
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(vm.as_bytes());
    hasher.update([0]);
    hasher.update(role.as_bytes());
    hasher.update(generation.to_le_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let rendered = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    ResourceUid::parse(rendered).expect("stable provider uid")
}

fn stable_token(value: &str) -> String {
    let valid = !value.is_empty()
        && value.len() <= 63
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        let digest = Sha256::digest(value.as_bytes());
        return format!(
            "process-{:02x}{:02x}{:02x}{:02x}",
            digest[0], digest[1], digest[2], digest[3]
        );
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_uids_are_uuid_v4_shaped_and_repeatable() {
        let first = stable_uid("process", "corp-vm", "ch-runner", 7);
        let second = stable_uid("process", "corp-vm", "ch-runner", 7);
        let other = stable_uid("process", "corp-vm", "audio", 7);
        assert_eq!(first, second);
        assert_ne!(first, other);
    }

    #[test]
    fn stable_tokens_close_invalid_bundle_names_without_paths() {
        assert_eq!(stable_token("audio-sidecar"), "audio-sidecar");
        assert!(stable_token("/var/lib/d2b/audio").starts_with("process-"));
        assert!(stable_token("UpperCase").starts_with("process-"));
    }

    #[test]
    fn production_composition_registers_only_fixed_process_providers() {
        assert_eq!(
            ProductionProcessProviders::provider_names(),
            &["system-minijail", "system-systemd"]
        );
    }

    #[test]
    fn managed_resource_finalization_requires_the_current_resource_identity() {
        let resource_ref = ResourceRef::parse("Process/worker").expect("resource ref");
        let uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid");
        let provider_ref = ResourceRef::parse("Provider/system-minijail").expect("provider ref");
        let managed = ManagedResource {
            provider: ManagedProvider::Minijail,
            identity: ProcessIdentityDigest::from_bytes([7; 32]),
            uid: uid.clone(),
            generation: ResourceGeneration::new(4).expect("generation"),
        };
        let context = ProcessResourceContext::new(
            &resource_ref,
            &uid,
            ResourceGeneration::new(4).expect("generation"),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
        );
        assert!(resource_identity_matches(&managed, context));
        let stale_context = ProcessResourceContext::new(
            &resource_ref,
            &uid,
            ResourceGeneration::new(3).expect("generation"),
            &provider_ref,
            ControllerGeneration::new(1).expect("controller generation"),
        );
        assert!(!resource_identity_matches(&managed, stale_context));
    }

    #[test]
    fn drain_retry_policy_distinguishes_transient_and_permanent_failures() {
        assert!(retryable_stop_error("stop-failed"));
        assert!(retryable_stop_error("process-fate-unknown"));
        assert!(!retryable_stop_error("identity-mismatch"));
        assert!(!retryable_stop_error("permission-denied"));
    }
}
