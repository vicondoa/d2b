//! Daemon-owned composition of the fixed process Providers.
//!
//! The Provider crates remain pure controllers: they receive only the
//! core-owned effect ports. This module is the one production seam that
//! constructs those ports from the authenticated broker transport and the
//! trusted bundle. No Provider receives a broker socket or a bundle resolver.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use d2b_contracts::broker_wire::BrokerCallerRole;
use d2b_contracts::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid,
};
use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_core::{
    bundle_resolver::BundleResolver,
    processes::{ProcessNode, ProcessRole},
};
use d2b_process_conformance::{
    AdoptionOutcome, CompiledDigests, ConfigurationDigest, IdentityBinding, LaunchTicket,
    OperationBinding, ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort,
    ProcessProvider, ProcessStatusReport, ReadinessExpectation, StopClass,
};
use d2b_provider_supervisor::{
    BrokerProcessBackend, BrokerSystemdEffectOwner, BundleBackedLaunchResolver,
    ProviderSupervisor, SystemdProcessBackend,
};
use d2b_provider_system_minijail::MinijailProcessProvider;
use d2b_provider_system_systemd::SystemdProcessProvider;
use sha2::{Digest, Sha256};

/// The fixed process Provider names wired by the daemon.
pub const FIXED_PROCESS_PROVIDER_NAMES: [&str; 2] = ["system-minijail", "system-systemd"];

type BrokerProcessSupervisor =
    ProviderSupervisor<BrokerProcessBackend<BundleBackedLaunchResolver>>;
type BrokerSystemdSupervisor =
    ProviderSupervisor<SystemdProcessBackend<BrokerSystemdEffectOwner>>;

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

impl crate::supervisor::readiness_liveness::LivenessProbe for ProviderLivenessProbe {
    fn probe(&self) -> crate::supervisor::readiness_liveness::RunnerLiveness {
        match crate::block_on_future(self.providers.probe_node(&self.vm, &self.node)) {
            Ok(ProviderLiveness::Alive) => {
                crate::supervisor::readiness_liveness::RunnerLiveness::Alive
            }
            Ok(ProviderLiveness::Exited) => {
                crate::supervisor::readiness_liveness::RunnerLiveness::Exited(None)
            }
            Ok(ProviderLiveness::Unknown) | Err(_) => {
                crate::supervisor::readiness_liveness::RunnerLiveness::Unknown
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
        Self {
            minijail: MinijailProcessProvider::new(ProviderSupervisor::new(minijail_backend)),
            systemd: SystemdProcessProvider::new(ProviderSupervisor::new(
                SystemdProcessBackend::new(systemd_owner),
            )),
            bundle,
            managed: Mutex::new(BTreeMap::new()),
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
        !matches!(node.role, ProcessRole::SwtpmPreStartFlush)
            && Self::supports_node(node)
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
            ManagedProvider::Systemd => self
                .systemd
                .launch(&ticket)
                .await
                .map_err(provider_error)?,
        };
        self.remember(vm, node, report.identity)?;
        Ok(ProviderLaunch {
            identity: report.identity,
        })
    }

    /// Adopt one trusted process node after a daemon restart.
    pub async fn adopt_node(
        &self,
        vm: &str,
        node: &ProcessNode,
    ) -> Result<ProviderAdoption, String> {
        let ticket = self.ticket(vm, node)?;
        let outcome = match self.provider_for(node) {
            ManagedProvider::Minijail => self
                .minijail
                .adopt(&ticket)
                .await
                .map_err(provider_error)?,
            ManagedProvider::Systemd => self
                .systemd
                .adopt(&ticket)
                .await
                .map_err(provider_error)?,
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

    fn forget(&self, vm: &str, node: &ProcessNode) {
        if let Ok(mut managed) = self.managed.lock() {
            managed.remove(&(vm.to_owned(), Self::tracked_role_id(node)));
        }
    }

    async fn stop_identity(
        &self,
        managed: ManagedProcess,
        class: StopClass,
    ) -> Result<(), String> {
        match managed.provider {
            ManagedProvider::Minijail => self
                .minijail
                .stop(&managed.identity, class)
                .await
                .map_err(provider_error),
            ManagedProvider::Systemd => self
                .systemd
                .stop(&managed.identity, class)
                .await
                .map_err(provider_error),
        }
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
        build_ticket(
            &self.bundle,
            vm,
            node,
            self.provider_for(node),
            timeout,
        )
            .map_err(|error| format!("provider-ticket:{}", error.code()))
    }
}

fn provider_error(error: ProcessConformanceError) -> String {
    error.code().to_owned()
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
        ResourceGeneration::new(generation)
            .map_err(|_| ProcessConformanceError::InvalidTicket)?,
        ControllerGeneration::new(1)
            .map_err(|_| ProcessConformanceError::InvalidTicket)?,
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
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
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
}
