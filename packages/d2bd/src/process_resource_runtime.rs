//! Daemon-owned reconciliation for generic `Process` resources.
//!
//! The fixed process Providers are composed once by `d2bd`; this module is
//! only the Zone-scoped durable-resource adapter. It relists and watches the
//! store, resolves typed specs, and routes every lifecycle effect through the
//! already composed Provider supervisors.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, ResourceEnvelope, ResourcePhase, ResourceRef,
    ResourceTypeName, ZoneId, ZoneRevision,
    process::{EphemeralProcessSpec, ProcessSpec, RestartClass},
};
use d2b_process_conformance::GuestExecutionBinding;
use d2b_resource_api::watch::ResourceWatch;
use d2b_resource_api::{
    RedbBackend, ResourceApiClient, ResourceStoreBackend,
    service::{UnavailableUpgradeDispatcher, UpgradeDispatcher},
};
use d2b_resource_store::{
    StoreGetRequest, StoreListRequest, StoreOperationContext, StoreProjection,
    StoreWatchRequest, StoreErrorKind, StoredResource,
};
use d2b_resource_store_redb::RedbResourceStore;
use sha2::{Digest, Sha256};

use crate::process_provider_runtime::{
    GUEST_EXECUTION_UNAVAILABLE, ProcessResourceContext, ProductionProcessProviders,
    ProviderAdoption, ProviderLiveness,
};
use d2bd_runtime::target_runtime::DaemonMode;

const PROCESS_TYPE: &str = "Process";
const EPHEMERAL_PROCESS_TYPE: &str = "EphemeralProcess";
const MINIJAIL_PROVIDER: &str = "system-minijail";
const SYSTEMD_PROVIDER: &str = "system-systemd";
const PROCESS_RUNTIME_FINALIZER: &str = "process-runtime.d2bus.org/cleanup";
const WAYLAND_SESSION_TYPE: &str = "display-wayland.d2bus.org.WaylandSession";
const WAYLAND_SESSION_FINALIZER: &str = "display-wayland.d2bus.org/proxy-stopped";
pub(crate) const PROCESS_RESTART_ANNOTATION: &str = "d2b.d2bus.org/restart-generation";

/// Stable failures for the daemon-owned generic process path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessResourceRuntimeError {
    /// A durable resource did not decode as the closed Process contract.
    InvalidResource,
    /// The resource selected a Provider not owned by this runtime.
    UnsupportedProvider,
    /// The trusted bundle did not contain the requested template binding.
    TemplateUnavailable,
    /// A process identity was ambiguous during adoption or stop.
    IdentityAmbiguous,
    /// A Provider effect failed.
    ProviderEffect,
    /// The durable store could not be listed or watched.
    Store,
}

impl core::fmt::Display for ProcessResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "process-resource-invalid",
            Self::UnsupportedProvider => "process-resource-provider-unsupported",
            Self::TemplateUnavailable => "process-resource-template-unavailable",
            Self::IdentityAmbiguous => "process-resource-identity-ambiguous",
            Self::ProviderEffect => "process-resource-provider-effect-failed",
            Self::Store => "process-resource-store-failed",
        })
    }
}

impl std::error::Error for ProcessResourceRuntimeError {}

#[async_trait::async_trait]
pub(crate) trait ProcessResourceClient: Send + Sync {
    async fn update_status(
        &self,
        request: wire::UpdateStatusRequest,
    ) -> wire::UpdateStatusResponse;

    async fn update_finalizers(
        &self,
        request: wire::UpdateFinalizersRequest,
    ) -> wire::UpdateFinalizersResponse;

    async fn delete(&self, request: wire::DeleteRequest) -> wire::DeleteResponse;
}

#[async_trait::async_trait]
impl<S, U> ProcessResourceClient for ResourceApiClient<S, U>
where
    S: ResourceStoreBackend + 'static,
    U: UpgradeDispatcher + 'static,
{
    async fn update_status(
        &self,
        request: wire::UpdateStatusRequest,
    ) -> wire::UpdateStatusResponse {
        ResourceApiClient::update_status(self, request).await
    }

    async fn update_finalizers(
        &self,
        request: wire::UpdateFinalizersRequest,
    ) -> wire::UpdateFinalizersResponse {
        ResourceApiClient::update_finalizers(self, request).await
    }

    async fn delete(&self, request: wire::DeleteRequest) -> wire::DeleteResponse {
        ResourceApiClient::delete(self, request).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DesiredProcess {
    Process(ProcessSpec),
    Ephemeral(EphemeralProcessSpec),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesiredRecord {
    resource: StoredResource,
    provider_ref: ResourceRef,
    process: DesiredProcess,
}

impl DesiredRecord {
    fn key(&self) -> ResourceRef {
        self.resource.resource_ref.clone()
    }

    fn is_running(&self) -> bool {
        match &self.process {
            DesiredProcess::Process(spec) => {
                spec.desired_lifecycle()
                    == d2b_contracts_resource::v3::process::DesiredLifecycle::Running
            }
            DesiredProcess::Ephemeral(_) => true,
        }
    }

    fn same_desired_state(&self, other: &Self) -> bool {
        self.resource.zone == other.resource.zone
            && self.resource.resource_ref == other.resource.resource_ref
            && self.resource.uid == other.resource.uid
            && self.resource.generation == other.resource.generation
            && restart_annotation(&self.resource) == restart_annotation(&other.resource)
            && self.provider_ref == other.provider_ref
            && self.process == other.process
    }

    fn owner_ref(&self) -> Option<ResourceRef> {
        let CanonicalJsonValue::Object(root) =
            CanonicalJsonValue::parse(&self.resource.canonical_json).ok()?
        else {
            return None;
        };
        let CanonicalJsonValue::Object(metadata) = root.get("metadata")? else {
            return None;
        };
        let CanonicalJsonValue::String(owner) = metadata.get("ownerRef")? else {
            return None;
        };
        ResourceRef::parse(owner).ok()
    }

    fn deletion_requested(&self) -> bool {
        metadata_value(&self.resource, "deletionRequestedAt")
            .is_some_and(|value| !matches!(value, CanonicalJsonValue::Null))
    }

    fn has_runtime_finalizer(&self) -> bool {
        metadata_value(&self.resource, "finalizers").is_some_and(|value| {
            matches!(
                value,
                CanonicalJsonValue::Array(values)
                    if values.iter().any(|value| {
                        matches!(value, CanonicalJsonValue::String(value) if value == PROCESS_RUNTIME_FINALIZER)
                    })
            )
        })
    }
}

/// Durable generic process registry for one Zone.
pub(crate) struct ProcessResourceRuntime {
    zone: ZoneId,
    target: Option<ResourceRef>,
    providers: Arc<ProductionProcessProviders>,
    records: BTreeMap<ResourceRef, DesiredRecord>,
    terminal: BTreeSet<ResourceRef>,
    terminal_failed: BTreeSet<ResourceRef>,
    restart_counts: BTreeMap<ResourceRef, u32>,
    started_at: BTreeMap<ResourceRef, Instant>,
    completed_at: BTreeMap<ResourceRef, Instant>,
    next_restart_at: BTreeMap<ResourceRef, Instant>,
    controller_generation: ControllerGeneration,
    guest_execution: Option<GuestExecutionBinding>,
    /// Optional owner and target selector for resources using a shared Host
    /// execution reference, retained across relist/watch passes.
    target_owner_ref: Option<ResourceRef>,
    target_ref: Option<ResourceRef>,
    status_client: Option<Arc<dyn ProcessResourceClient>>,
}

impl core::fmt::Debug for ProcessResourceRuntime {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProcessResourceRuntime")
            .field("zone", &self.zone)
            .field("record_count", &self.records.len())
            .finish()
    }
}

fn scoped_target_ref(
    record: &DesiredRecord,
    target_owner_ref: Option<&ResourceRef>,
    target_ref: Option<&ResourceRef>,
) -> Option<ResourceRef> {
    match (target_owner_ref, target_ref, record.owner_ref()) {
        (Some(expected_owner), Some(target), Some(owner)) if expected_owner == &owner => {
            Some(target.clone())
        }
        _ => None,
    }
}

impl ProcessResourceRuntime {
    /// Construct a registry over the daemon-owned fixed Providers.
    pub(crate) fn new(zone: ZoneId, providers: Arc<ProductionProcessProviders>) -> Self {
        Self::new_for_target(zone, providers, None)
    }

    pub(crate) fn new_for_target(
        zone: ZoneId,
        providers: Arc<ProductionProcessProviders>,
        target: Option<ResourceRef>,
    ) -> Self {
        Self {
            zone,
            target,
            providers,
            records: BTreeMap::new(),
            terminal: BTreeSet::new(),
            terminal_failed: BTreeSet::new(),
            restart_counts: BTreeMap::new(),
            started_at: BTreeMap::new(),
            completed_at: BTreeMap::new(),
            next_restart_at: BTreeMap::new(),
            controller_generation: ControllerGeneration::new(1)
                .expect("controller generation one is valid"),
            guest_execution: None,
            target_owner_ref: None,
            target_ref: None,
            status_client: None,
        }
    }

    pub(crate) fn set_controller_generation(&mut self, generation: ControllerGeneration) {
        self.controller_generation = generation;
    }

    pub(crate) fn set_guest_execution_binding(&mut self, binding: GuestExecutionBinding) {
        self.guest_execution = Some(binding);
    }

    pub(crate) fn set_target_scope(
        &mut self,
        target_owner_ref: Option<ResourceRef>,
        target_ref: Option<ResourceRef>,
    ) {
        self.target_owner_ref = target_owner_ref;
        self.target_ref = target_ref;
    }

    pub(crate) fn set_status_client<C>(&mut self, status_client: Arc<C>)
    where
        C: ProcessResourceClient + 'static,
    {
        self.status_client = Some(status_client);
    }

    fn context<'a>(&self, record: &'a DesiredRecord) -> ProcessResourceContext<'a> {
        let target_ref =
            scoped_target_ref(record, self.target_owner_ref.as_ref(), self.target_ref.as_ref());
        ProcessResourceContext::new(
            &record.resource.resource_ref,
            &record.resource.uid,
            record.resource.generation,
            record.resource.revision,
            &record.provider_ref,
            self.controller_generation,
            target_ref,
        )
        .with_guest_execution(self.guest_execution.as_ref())
    }

    /// Reconcile a complete durable Process/EphemeralProcess snapshot.
    pub(crate) async fn reconcile(
        &mut self,
        snapshot: Vec<StoredResource>,
    ) -> Result<(), ProcessResourceRuntimeError> {
        let desired = decode_snapshot(&self.zone, self.target.as_ref(), snapshot, self.providers.mode())?;
        let desired_keys = desired.keys().cloned().collect::<BTreeSet<_>>();
        let removed = self
            .records
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in removed {
            if let Some(record) = self.records.get(&key).cloned() {
                self.stop_record(&record).await?;
                self.records.remove(&key);
            }

            self.terminal.remove(&key);
            self.terminal_failed.remove(&key);
            self.restart_counts.remove(&key);
            self.started_at.remove(&key);
            self.completed_at.remove(&key);
            self.next_restart_at.remove(&key);
        }

        for (key, mut record) in desired {
            let was_present = self.records.contains_key(&key);
            let replace = self
                .records
                .get(&key)
                .is_some_and(|current| !current.same_desired_state(&record));
            if replace {
                if let Some(current) = self.records.get(&key).cloned() {
                    self.stop_record(&current).await?;
                    self.records.remove(&key);
                }
                self.terminal.remove(&key);
                self.terminal_failed.remove(&key);
                self.restart_counts.remove(&key);
                self.started_at.remove(&key);
                self.completed_at.remove(&key);
                self.next_restart_at.remove(&key);
            }

            if !was_present && !replace && !self.providers.has_active_resource(&key) {
                match status_phase(&record.resource) {
                    Some(ResourcePhase::Succeeded) => {
                        self.terminal.insert(key.clone());
                        self.completed_at.insert(key.clone(), Instant::now());
                    }
                    Some(ResourcePhase::Failed) => {
                        self.terminal.insert(key.clone());
                        self.terminal_failed.insert(key.clone());
                        self.completed_at.insert(key.clone(), Instant::now());
                    }
                    _ => {}
                }
            }

            if !record.deletion_requested()
                && !record.has_runtime_finalizer()
                && !self.terminal.contains(&key)
            {
                record = self.ensure_finalizer(&record).await?;
            }

            if record.deletion_requested() {
                if !self.providers.has_active_resource(&key) {
                    match &record.process {
                        DesiredProcess::Process(spec) => {
                            let adoption = deletion_adoption(
                                self.providers
                                    .adopt_resource(self.context(&record), spec)
                                    .await,
                            )?;
                            match adoption {
                                ProviderAdoption::Quarantined(_) => {
                                    return Err(ProcessResourceRuntimeError::IdentityAmbiguous);
                                }
                                ProviderAdoption::Adopted(_) | ProviderAdoption::Absent => {}
                            }
                        }
                        DesiredProcess::Ephemeral(spec) => {
                            let adoption = deletion_adoption(
                                self.providers
                                    .adopt_ephemeral_resource(self.context(&record), spec)
                                    .await,
                            )?;
                            match adoption {
                                ProviderAdoption::Quarantined(_) => {
                                    return Err(ProcessResourceRuntimeError::IdentityAmbiguous);
                                }
                                ProviderAdoption::Adopted(_) | ProviderAdoption::Absent => {}
                            }
                        }
                    }
                }
                if self.providers.has_active_resource(&key) {
                    self.stop_record(&record).await?;
                }
                self.providers
                    .finalize_resource(self.context(&record))
                    .await
                    .map_err(map_provider_error)?;
                record = self
                    .publish_status(&record, ResourcePhase::Deleted, None)
                    .await?;
                record = self.remove_finalizer(&record).await?;
                self.terminal.insert(key.clone());
                self.records.insert(key, record);
                continue;
            }

            if self.terminal.contains(&key) {
                if self.ephemeral_ttl_elapsed(&key, &record) {
                    self.request_delete(&record).await?;
                }
                self.records.insert(key, record);
                continue;
            }

            if let DesiredProcess::Ephemeral(spec) = &record.process
                && self.providers.has_active_resource(&key)
                && self.started_at.get(&key).is_some_and(|started| {
                    started.elapsed() >= Duration::from_millis(spec.runtime_deadline().as_millis())
                })
            {
                self.stop_record(&record).await?;
                self.providers
                    .finalize_resource(self.context(&record))
                    .await
                    .map_err(map_provider_error)?;
                self.completed_at.insert(key.clone(), Instant::now());
                self.terminal.insert(key.clone());
                record = self
                    .publish_status(
                        &record,
                        ResourcePhase::Succeeded,
                        Some(OutcomeState::success(
                            "runtime-deadline",
                            "ephemeral process reached its runtime deadline",
                        )),
                    )
                    .await?;
                self.records.insert(key, record);
                continue;
            }

            if !record.is_running() {
                if self.providers.has_active_resource(&key) {
                    self.stop_record(&record).await?;
                    self.providers
                        .finalize_resource(self.context(&record))
                        .await
                        .map_err(map_provider_error)?;
                }
                record = self
                    .publish_status(&record, ResourcePhase::Succeeded, None)
                    .await?;
                self.records.insert(key.clone(), record);
                self.terminal.remove(&key);
                self.terminal_failed.remove(&key);
                self.restart_counts.remove(&key);
                self.started_at.remove(&key);
                self.completed_at.remove(&key);
                self.next_restart_at.remove(&key);
                continue;
            }

            if let Some(restart_at) = self.next_restart_at.get(&key).copied() {
                if Instant::now() < restart_at {
                    self.records.insert(key, record);
                    continue;
                }
                self.next_restart_at.remove(&key);
                match self.start_record(&record).await {
                    Ok(adopted) => {
                        self.started_at.insert(key.clone(), Instant::now());
                        record = self
                            .publish_status(
                                &record,
                                ResourcePhase::Ready,
                                Some(OutcomeState::ready(adopted)),
                            )
                            .await?;
                        self.records.insert(key, record);
                    }
                    Err(error) => {
                        self.handle_start_failure(key, record, error).await?;
                    }
                }
                continue;
            }

            if self
                .started_at
                .get(&key)
                .is_some_and(|started| restart_reset_due(&record.process, *started))
            {
                self.restart_counts.insert(key.clone(), 0);
            }

            if was_present && !replace {
                match self.probe_record(&record).await? {
                    ProviderLiveness::Alive => {}
                    ProviderLiveness::Unknown => {
                        self.terminal.insert(key.clone());
                        self.terminal_failed.insert(key.clone());
                        self.completed_at.insert(key.clone(), Instant::now());
                        record = self
                            .publish_status(
                                &record,
                                ResourcePhase::Failed,
                                Some(OutcomeState::failure(
                                    "identity-ambiguous",
                                    "provider identity could not be verified safely",
                                )),
                            )
                            .await?;
                        self.records.insert(key, record);
                    }
                    ProviderLiveness::Exited => {
                        self.providers
                            .finalize_resource(self.context(&record))
                            .await
                            .map_err(map_provider_error)?;
                        let restart = match &record.process {
                            DesiredProcess::Process(spec) => {
                                spec.restart_policy().class() != RestartClass::Never
                                    && spec.restart_policy().max_restarts().is_none_or(|max| {
                                        self.restart_counts.get(&key).copied().unwrap_or(0) < max
                                    })
                            }
                            DesiredProcess::Ephemeral(_) => false,
                        };
                        if restart {
                            let restart_count = self.restart_counts.entry(key.clone()).or_default();
                            *restart_count = restart_count.saturating_add(1);
                            let delay = restart_delay(&record.process, *restart_count);
                            self.next_restart_at
                                .insert(key.clone(), Instant::now() + delay);
                            record = self
                                .publish_status(
                                    &record,
                                    ResourcePhase::Degraded,
                                    Some(OutcomeState::retry(
                                        "process-exited",
                                        "process exited and is awaiting restart",
                                        delay,
                                    )),
                                )
                                .await?;
                            self.records.insert(key, record);
                        } else {
                            self.terminal.insert(key.clone());
                            self.completed_at.insert(key.clone(), Instant::now());
                            record = self
                                .publish_status(
                                    &record,
                                    ResourcePhase::Succeeded,
                                    Some(OutcomeState::success(
                                        "process-exited",
                                        "process reached a terminal exit",
                                    )),
                                )
                                .await?;
                            self.records.insert(key, record);
                        }
                    }
                }
                continue;
            }

            record = self
                .publish_status(&record, ResourcePhase::Pending, None)
                .await?;
            match self.start_record(&record).await {
                Ok(adopted) => {
                    self.started_at.insert(key.clone(), Instant::now());
                    record = self
                        .publish_status(
                            &record,
                            ResourcePhase::Ready,
                            Some(OutcomeState::ready(adopted)),
                        )
                        .await?;
                    self.records.insert(key, record);
                }
                Err(error) => {
                    self.handle_start_failure(key, record, error).await?;
                }
            }
        }
        Ok(())
    }

    async fn start_record(
        &self,
        record: &DesiredRecord,
    ) -> Result<bool, ProcessResourceRuntimeError> {
        let adoption = match &record.process {
            DesiredProcess::Process(spec)
                if spec.adoption_policy()
                    == d2b_contracts_resource::v3::process::AdoptionPolicy::NeverAdopt =>
            {
                ProviderAdoption::Absent
            }
            DesiredProcess::Process(spec) => self
                .providers
                .adopt_resource(self.context(record), spec)
                .await
                .map_err(map_provider_error)?,
            DesiredProcess::Ephemeral(spec) => self
                .providers
                .adopt_ephemeral_resource(self.context(record), spec)
                .await
                .map_err(map_provider_error)?,
        };
        match adoption {
            ProviderAdoption::Adopted(_) => Ok(true),
            ProviderAdoption::Quarantined(_) => Err(ProcessResourceRuntimeError::IdentityAmbiguous),
            ProviderAdoption::Absent => {
                match &record.process {
                    DesiredProcess::Process(spec) => self
                        .providers
                        .launch_resource(
                            self.context(record),
                            spec,
                            launch_timeout(&record.process),
                        )
                        .await
                        .map_err(map_provider_error)?,
                    DesiredProcess::Ephemeral(spec) => self
                        .providers
                        .launch_ephemeral_resource(
                            self.context(record),
                            spec,
                            launch_timeout(&record.process),
                        )
                        .await
                        .map_err(map_provider_error)?,
                };
                Ok(false)
            }
        }
    }

    async fn handle_start_failure(
        &mut self,
        key: ResourceRef,
        mut record: DesiredRecord,
        error: ProcessResourceRuntimeError,
    ) -> Result<(), ProcessResourceRuntimeError> {
        let identity_failure = matches!(
            error,
            ProcessResourceRuntimeError::IdentityAmbiguous
                | ProcessResourceRuntimeError::TemplateUnavailable
        );
        let restart = !identity_failure
            && matches!(
                &record.process,
                DesiredProcess::Process(spec)
                    if spec.restart_policy().class() != RestartClass::Never
                        && spec.restart_policy().max_restarts().is_none_or(|max| {
                            self.restart_counts.get(&key).copied().unwrap_or(0) < max
                        })
            );
        if restart {
            let restart_count = self.restart_counts.entry(key.clone()).or_default();
            *restart_count = restart_count.saturating_add(1);
            let delay = restart_delay(&record.process, *restart_count);
            self.next_restart_at
                .insert(key.clone(), Instant::now() + delay);
            record = self
                .publish_status(
                    &record,
                    ResourcePhase::Degraded,
                    Some(OutcomeState::retry(
                        "provider-start-failed",
                        "provider failed to start the process",
                        delay,
                    )),
                )
                .await?;
        } else {
            self.terminal.insert(key.clone());
            self.terminal_failed.insert(key.clone());
            self.completed_at.insert(key.clone(), Instant::now());
            record = self
                .publish_status(
                    &record,
                    ResourcePhase::Failed,
                    Some(OutcomeState::failure(
                        start_failure_code(error),
                        start_failure_message(error),
                    )),
                )
                .await?;
        }
        self.records.insert(key, record);
        Ok(())
    }

    async fn publish_status(
        &self,
        record: &DesiredRecord,
        phase: ResourcePhase,
        outcome: Option<OutcomeState>,
    ) -> Result<DesiredRecord, ProcessResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(record.clone());
        };
        update_status(
            client.as_ref(),
            record,
            phase,
            self.restart_counts
                .get(&record.resource.resource_ref)
                .copied()
                .unwrap_or(0),
            outcome,
        )
        .await
    }

    async fn ensure_finalizer(
        &self,
        record: &DesiredRecord,
    ) -> Result<DesiredRecord, ProcessResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(record.clone());
        };
        if record.has_runtime_finalizer() {
            return Ok(record.clone());
        }
        update_finalizers(client.as_ref(), record, true).await
    }

    async fn remove_finalizer(
        &self,
        record: &DesiredRecord,
    ) -> Result<DesiredRecord, ProcessResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(record.clone());
        };
        if !record.has_runtime_finalizer() {
            return Ok(record.clone());
        }
        update_finalizers(client.as_ref(), record, false).await
    }

    fn ephemeral_ttl_elapsed(&self, key: &ResourceRef, record: &DesiredRecord) -> bool {
        let DesiredProcess::Ephemeral(spec) = &record.process else {
            return false;
        };
        if self.status_client.is_none() {
            return false;
        }
        if self.terminal_failed.contains(key) && spec.incident_hold() {
            return false;
        }
        let Some(completed_at) = self.completed_at.get(key) else {
            return false;
        };
        let ttl = if self.terminal_failed.contains(key) {
            spec.failed_ttl().as_millis()
        } else {
            spec.successful_ttl().as_millis()
        };
        completed_at.elapsed() >= Duration::from_millis(ttl)
    }

    async fn request_delete(
        &self,
        record: &DesiredRecord,
    ) -> Result<(), ProcessResourceRuntimeError> {
        let Some(client) = &self.status_client else {
            return Ok(());
        };
        delete_resource(client.as_ref(), record).await
    }

    async fn probe_record(
        &self,
        record: &DesiredRecord,
    ) -> Result<ProviderLiveness, ProcessResourceRuntimeError> {
        let liveness = match &record.process {
            DesiredProcess::Process(spec) => self
                .providers
                .probe_resource(self.context(record), spec)
                .await
                .map_err(map_provider_error)?,
            DesiredProcess::Ephemeral(spec) => self
                .providers
                .probe_ephemeral_resource(self.context(record), spec)
                .await
                .map_err(map_provider_error)?,
        };
        Ok(liveness)
    }

    async fn stop_record(&self, record: &DesiredRecord) -> Result<(), ProcessResourceRuntimeError> {
        if !self
            .providers
            .has_active_resource(&record.resource.resource_ref)
        {
            return Ok(());
        }
        match &record.process {
            DesiredProcess::Process(spec) => self
                .providers
                .stop_resource(
                    self.context(record),
                    spec,
                    process_drain_timeout(spec),
                    Duration::from_secs(30),
                )
                .await
                .map_err(map_provider_error)?,
            DesiredProcess::Ephemeral(spec) => self
                .providers
                .stop_ephemeral_resource(
                    self.context(record),
                    spec,
                    Duration::from_secs(30),
                    Duration::from_secs(30),
                )
                .await
                .map_err(map_provider_error)?,
        };
        Ok(())
    }
}

fn launch_timeout(process: &DesiredProcess) -> Duration {
    match process {
        DesiredProcess::Process(_) => Duration::from_secs(30),
        DesiredProcess::Ephemeral(spec) => Duration::from_millis(spec.start_deadline().as_millis()),
    }
}

fn process_drain_timeout(spec: &ProcessSpec) -> Duration {
    Duration::from_millis(spec.drain_timeout().as_millis())
}

#[derive(Debug, Clone)]
struct OutcomeState {
    code: &'static str,
    message: &'static str,
    retryable: bool,
    retry_after_ms: Option<u32>,
    adopted: Option<bool>,
}

impl OutcomeState {
    fn ready(adopted: bool) -> Self {
        Self {
            code: "process-ready",
            message: if adopted {
                "process was adopted by its Provider"
            } else {
                "process was launched by its Provider"
            },
            retryable: false,
            retry_after_ms: None,
            adopted: Some(adopted),
        }
    }

    fn success(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
            retry_after_ms: None,
            adopted: None,
        }
    }

    fn failure(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            retryable: false,
            retry_after_ms: None,
            adopted: None,
        }
    }

    fn retry(code: &'static str, message: &'static str, delay: Duration) -> Self {
        let retry_after_ms = u32::try_from(delay.as_millis()).ok();
        Self {
            code,
            message,
            retryable: true,
            retry_after_ms,
            adopted: None,
        }
    }
}

fn metadata_value(resource: &StoredResource, key: &str) -> Option<CanonicalJsonValue> {
    let value = CanonicalJsonValue::parse(&resource.canonical_json).ok()?;
    let CanonicalJsonValue::Object(root) = value else {
        return None;
    };
    let CanonicalJsonValue::Object(metadata) = root.get("metadata")? else {
        return None;
    };
    metadata.get(key).cloned()
}

fn status_phase(resource: &StoredResource) -> Option<ResourcePhase> {
    let value = CanonicalJsonValue::parse(&resource.canonical_json).ok()?;
    let CanonicalJsonValue::Object(root) = value else {
        return None;
    };
    let CanonicalJsonValue::Object(status) = root.get("status")? else {
        return None;
    };
    let CanonicalJsonValue::String(phase) = status.get("phase")? else {
        return None;
    };
    match phase.as_str() {
        "Pending" => Some(ResourcePhase::Pending),
        "Ready" => Some(ResourcePhase::Ready),
        "Succeeded" => Some(ResourcePhase::Succeeded),
        "Degraded" => Some(ResourcePhase::Degraded),
        "Failed" => Some(ResourcePhase::Failed),
        "Deleted" => Some(ResourcePhase::Deleted),
        "Unknown" => Some(ResourcePhase::Unknown),
        _ => None,
    }
}

fn restart_reset_due(process: &DesiredProcess, started: Instant) -> bool {
    match process {
        DesiredProcess::Process(spec) => {
            started.elapsed()
                >= Duration::from_millis(spec.restart_policy().reset_after().as_millis())
        }
        DesiredProcess::Ephemeral(_) => false,
    }
}

fn restart_delay(process: &DesiredProcess, restart_count: u32) -> Duration {
    let DesiredProcess::Process(spec) = process else {
        return Duration::ZERO;
    };
    let policy = spec.restart_policy();
    let base = policy.backoff_base().as_millis();
    let max = policy.backoff_max().as_millis();
    let multiplier = u64::from(policy.backoff_multiplier_milli());
    let mut delay = base;
    for _ in 1..restart_count {
        delay = delay
            .saturating_mul(multiplier)
            .saturating_div(1_000)
            .min(max);
    }
    Duration::from_millis(delay.min(max))
}

fn start_failure_code(error: ProcessResourceRuntimeError) -> &'static str {
    match error {
        ProcessResourceRuntimeError::TemplateUnavailable => "template-unavailable",
        ProcessResourceRuntimeError::IdentityAmbiguous => "identity-ambiguous",
        ProcessResourceRuntimeError::UnsupportedProvider => "provider-unsupported",
        ProcessResourceRuntimeError::InvalidResource => "resource-invalid",
        ProcessResourceRuntimeError::ProviderEffect => "provider-start-failed",
        ProcessResourceRuntimeError::Store => "store-failed",
    }
}

fn start_failure_message(error: ProcessResourceRuntimeError) -> &'static str {
    match error {
        ProcessResourceRuntimeError::TemplateUnavailable => {
            "the trusted process template binding is unavailable"
        }
        ProcessResourceRuntimeError::IdentityAmbiguous => {
            "the process identity could not be verified safely"
        }
        ProcessResourceRuntimeError::UnsupportedProvider => {
            "the process Provider is not owned by the daemon"
        }
        ProcessResourceRuntimeError::InvalidResource => "the process resource is invalid",
        ProcessResourceRuntimeError::ProviderEffect => "the Provider failed to start the process",
        ProcessResourceRuntimeError::Store => "the durable resource store failed",
    }
}

fn phase_json(phase: ResourcePhase) -> CanonicalJsonValue {
    CanonicalJsonValue::String(
        match phase {
            ResourcePhase::Pending => "Pending",
            ResourcePhase::Ready => "Ready",
            ResourcePhase::Succeeded => "Succeeded",
            ResourcePhase::Degraded => "Degraded",
            ResourcePhase::Failed => "Failed",
            ResourcePhase::Deleted => "Deleted",
            ResourcePhase::Unknown => "Unknown",
        }
        .to_owned(),
    )
}

fn now_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seconds = millis / 1_000;
    let day = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let (year, month, day_of_month) = civil_from_days(day as i64);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!(
        "{year:04}-{month:02}-{day_of_month:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        millis % 1_000
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

async fn update_status(
    client: &dyn ProcessResourceClient,
    record: &DesiredRecord,
    phase: ResourcePhase,
    restart_count: u32,
    outcome: Option<OutcomeState>,
) -> Result<DesiredRecord, ProcessResourceRuntimeError> {
    let canonical = status_payload(record, phase, restart_count, outcome)?;
    let envelope = ResourceEnvelope::from_json(&canonical)
        .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?;
    let digest = envelope
        .digest()
        .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?;
    let mut resource = wire::ResourceEnvelopeBytes::new();
    resource.identity = protobuf::MessageField::some(resource_identity(record));
    resource.canonical_json = canonical;
    resource.payload_digest = digest;

    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_STATUS);
    mutation.target = protobuf::MessageField::some(resource_identity(record));
    mutation.precondition = protobuf::MessageField::some(exact_precondition(record));
    mutation.resource = protobuf::MessageField::some(resource);
    let operation = format!(
        "process-runtime-status-{}-{}",
        record.key().to_canonical_string(),
        record.resource.revision.get()
    );
    let mut request = wire::UpdateStatusRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_status(request).await;
    if response.error.is_some() {
        return Err(ProcessResourceRuntimeError::Store);
    }
    let response_resource = response
        .resource
        .as_ref()
        .ok_or(ProcessResourceRuntimeError::Store)?;
    let mut updated = record.clone();
    updated.resource.canonical_json = response_resource.canonical_json.clone();
    updated.resource.payload_digest = response_resource.payload_digest.clone();
    updated.resource.revision = ZoneRevision::new(response.revision);
    Ok(updated)
}

fn status_payload(
    record: &DesiredRecord,
    phase: ResourcePhase,
    restart_count: u32,
    outcome: Option<OutcomeState>,
) -> Result<Vec<u8>, ProcessResourceRuntimeError> {
    let mut value = CanonicalJsonValue::parse(&record.resource.canonical_json)
        .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(ProcessResourceRuntimeError::InvalidResource);
    };
    let Some(CanonicalJsonValue::Object(status)) = root.get_mut("status") else {
        return Err(ProcessResourceRuntimeError::InvalidResource);
    };
    let now = now_timestamp();
    status.insert("phase".to_owned(), phase_json(phase));
    status.insert(
        "observedGeneration".to_owned(),
        CanonicalJsonValue::Integer(record.resource.generation.get() as i64),
    );
    status.insert(
        "lastReconciledAt".to_owned(),
        CanonicalJsonValue::String(now.clone()),
    );
    if phase == ResourcePhase::Ready
        && status
            .get("startedAt")
            .is_none_or(|value| matches!(value, CanonicalJsonValue::Null))
    {
        status.insert(
            "startedAt".to_owned(),
            CanonicalJsonValue::String(now.clone()),
        );
    }
    if matches!(
        phase,
        ResourcePhase::Succeeded | ResourcePhase::Failed | ResourcePhase::Deleted
    ) {
        status.insert(
            "completedAt".to_owned(),
            CanonicalJsonValue::String(now.clone()),
        );
    }
    status.insert(
        "outcome".to_owned(),
        outcome
            .as_ref()
            .map(|outcome| {
                let mut result = BTreeMap::new();
                result.insert(
                    "code".to_owned(),
                    CanonicalJsonValue::String(outcome.code.to_owned()),
                );
                result.insert(
                    "message".to_owned(),
                    CanonicalJsonValue::String(outcome.message.to_owned()),
                );
                result.insert(
                    "retryable".to_owned(),
                    CanonicalJsonValue::Bool(outcome.retryable),
                );
                result.insert(
                    "occurredAt".to_owned(),
                    CanonicalJsonValue::String(now.clone()),
                );
                if let Some(retry_after_ms) = outcome.retry_after_ms {
                    result.insert(
                        "retryAfterMs".to_owned(),
                        CanonicalJsonValue::Integer(i64::from(retry_after_ms)),
                    );
                }
                CanonicalJsonValue::Object(result)
            })
            .unwrap_or(CanonicalJsonValue::Null),
    );
    let Some(CanonicalJsonValue::Object(resource_status)) = status.get_mut("resource") else {
        return Err(ProcessResourceRuntimeError::InvalidResource);
    };
    resource_status.insert(
        "provider".to_owned(),
        CanonicalJsonValue::String(record.provider_ref.to_canonical_string()),
    );
    resource_status.insert(
        "restartCount".to_owned(),
        CanonicalJsonValue::Integer(i64::from(restart_count)),
    );
    if let Some(adopted) = outcome.as_ref().and_then(|outcome| outcome.adopted) {
        resource_status.insert("adopted".to_owned(), CanonicalJsonValue::Bool(adopted));
    }
    if let Some(CanonicalJsonValue::Object(update)) = status.get_mut("update") {
        update.insert(
            "observedGeneration".to_owned(),
            CanonicalJsonValue::Integer(record.resource.generation.get() as i64),
        );
        update.insert("lastAssessedAt".to_owned(), CanonicalJsonValue::String(now));
    }
    let canonical = value.to_canonical_bytes();
    ResourceEnvelope::from_json(&canonical)
        .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?;
    Ok(canonical)
}

async fn update_finalizers(
    client: &dyn ProcessResourceClient,
    record: &DesiredRecord,
    add: bool,
) -> Result<DesiredRecord, ProcessResourceRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind =
        protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
    mutation.target = protobuf::MessageField::some(resource_identity(record));
    mutation.precondition = protobuf::MessageField::some(exact_precondition(record));
    if add {
        mutation
            .add_finalizers
            .push(PROCESS_RUNTIME_FINALIZER.to_owned());
    } else {
        mutation
            .remove_finalizers
            .push(PROCESS_RUNTIME_FINALIZER.to_owned());
    }
    let operation = format!(
        "process-runtime-finalizer-{}-{}-{}",
        record.key().to_canonical_string(),
        record.resource.revision.get(),
        if add { "add" } else { "remove" }
    );
    let mut request = wire::UpdateFinalizersRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_finalizers(request).await;
    if response.error.is_some() {
        return Err(ProcessResourceRuntimeError::Store);
    }
    let response_resource = response
        .resource
        .as_ref()
        .ok_or(ProcessResourceRuntimeError::Store)?;
    let mut updated = record.clone();
    updated.resource.canonical_json = response_resource.canonical_json.clone();
    updated.resource.payload_digest = response_resource.payload_digest.clone();
    updated.resource.revision = ZoneRevision::new(response.revision);
    Ok(updated)
}

async fn delete_resource(
    client: &dyn ProcessResourceClient,
    record: &DesiredRecord,
) -> Result<(), ProcessResourceRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = protobuf::MessageField::some(resource_identity(record));
    mutation.precondition = protobuf::MessageField::some(exact_precondition(record));
    let operation = format!(
        "process-runtime-delete-{}-{}",
        record.key().to_canonical_string(),
        record.resource.revision.get()
    );
    let mut request = wire::DeleteRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.delete(request).await;
    if response.error.is_some() {
        return Err(ProcessResourceRuntimeError::Store);
    }
    Ok(())
}

fn resource_identity(record: &DesiredRecord) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = record.resource.zone.to_canonical_string();
    identity.resource_type = record
        .resource
        .resource_ref
        .resource_type()
        .to_canonical_string();
    identity.name = record.resource.resource_ref.name().to_canonical_string();
    identity.uid = Some(record.resource.uid.as_str().to_owned());
    identity.generation = Some(record.resource.generation.get());
    identity.revision = Some(record.resource.revision.get());
    identity
}

fn exact_precondition(record: &DesiredRecord) -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(record.resource.revision.get());
    precondition.expected_uid = Some(record.resource.uid.as_str().to_owned());
    precondition
}

fn request_meta(operation: &str) -> wire::RequestMeta {
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation.to_owned();
    meta.idempotency_key = operation.to_owned();
    meta.correlation_id = operation.to_owned();
    meta.trace_id = operation.to_owned();
    meta.deadline_ms = 10_000;
    meta
}

fn map_provider_error(error: String) -> ProcessResourceRuntimeError {
    if error.contains("template-not-found") {
        ProcessResourceRuntimeError::TemplateUnavailable
    } else if error.contains("quarantined")
        || error.contains("identity")
        || error.contains("ambiguous")
    {
        ProcessResourceRuntimeError::IdentityAmbiguous
    } else {
        ProcessResourceRuntimeError::ProviderEffect
    }
}

fn deletion_adoption(
    result: Result<ProviderAdoption, String>,
) -> Result<ProviderAdoption, ProcessResourceRuntimeError> {
    match result {
        Ok(adoption) => Ok(adoption),
        Err(error) if error == GUEST_EXECUTION_UNAVAILABLE => {
            Err(ProcessResourceRuntimeError::ProviderEffect)
        }
        Err(error) => Err(map_provider_error(error)),
    }
}

fn decode_snapshot(
    zone: &ZoneId,
    target: Option<&ResourceRef>,
    resources: Vec<StoredResource>,
    mode: DaemonMode,
) -> Result<BTreeMap<ResourceRef, DesiredRecord>, ProcessResourceRuntimeError> {
    let mut desired = BTreeMap::new();
    for resource in resources {
        if resource.zone != *zone {
            return Err(ProcessResourceRuntimeError::InvalidResource);
        }
        let resource_type = resource.resource_ref.resource_type().as_str();
        let envelope = ResourceEnvelope::from_json(&resource.canonical_json)
            .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?;
        let provider_ref = envelope
            .spec()
            .provider_ref()
            .cloned()
            .ok_or(ProcessResourceRuntimeError::InvalidResource)?;
        let execution_ref = envelope
            .spec()
            .base()
            .get("executionRef")
            .and_then(|value| match value {
                CanonicalJsonValue::String(value) => ResourceRef::parse(value).ok(),
                _ => None,
            })
            .ok_or(ProcessResourceRuntimeError::InvalidResource)?;
        let target_matches = if let Some(target) = target {
            execution_ref == *target
        } else {
            execution_ref.resource_type().as_str() == "Host"
        };
        if !target_matches {
            continue;
        }
        if provider_ref.resource_type().as_str() != "Provider"
            || !matches!(
                provider_ref.name().as_str(),
                MINIJAIL_PROVIDER | SYSTEMD_PROVIDER
            )
        {
            return Err(ProcessResourceRuntimeError::UnsupportedProvider);
        }
        let process = match resource_type {
            PROCESS_TYPE => DesiredProcess::Process(
                serde_json::from_slice(&envelope.spec().base().to_canonical_bytes())
                    .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?,
            ),
            EPHEMERAL_PROCESS_TYPE => DesiredProcess::Ephemeral(
                serde_json::from_slice(&envelope.spec().base().to_canonical_bytes())
                    .map_err(|_| ProcessResourceRuntimeError::InvalidResource)?,
            ),
            _ => continue,
        };
        if mode == DaemonMode::Host
            && match &process {
                DesiredProcess::Process(spec) => {
                    spec.execution().execution_ref().resource_type().as_str() == "Guest"
                }
                DesiredProcess::Ephemeral(spec) => {
                    spec.execution().execution_ref().resource_type().as_str() == "Guest"
                }
            }
        {
            // A Host Process controller cannot reconcile a Guest-local child.
            // Leave the intent pending for the authenticated Guest controller
            // rather than claiming failure or running it through host effects.
            continue;
        }
        let record = DesiredRecord {
            resource: resource.clone(),
            provider_ref,
            process,
        };
        if desired.insert(record.key(), record).is_some() {
            return Err(ProcessResourceRuntimeError::InvalidResource);
        }
    }
    Ok(desired)
}

fn restart_annotation(resource: &StoredResource) -> Option<String> {
    let value = CanonicalJsonValue::parse(&resource.canonical_json).ok()?;
    let CanonicalJsonValue::Object(root) = value else {
        return None;
    };
    let CanonicalJsonValue::Object(metadata) = root.get("metadata")? else {
        return None;
    };
    let CanonicalJsonValue::Object(annotations) = metadata.get("annotations")? else {
        return None;
    };
    match annotations.get(PROCESS_RESTART_ANNOTATION) {
        Some(CanonicalJsonValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

/// Build the generic Process relist request.
pub(crate) fn process_list_request(zone: &ZoneId) -> StoreListRequest {
    StoreListRequest {
        operation: StoreOperationContext {
            operation_id: "process-resource-reconcile".to_owned(),
            idempotency_key: None,
            correlation_id: "process-resource-reconcile".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(PROCESS_TYPE).expect("static Process type"),
            ResourceTypeName::parse(EPHEMERAL_PROCESS_TYPE).expect("static EphemeralProcess type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 256,
        cursor: None,
        projection: StoreProjection::Full,
    }
}

/// Build the generic Process watch request.
pub(crate) fn process_watch_request(zone: &ZoneId) -> StoreWatchRequest {
    StoreWatchRequest {
        operation: StoreOperationContext {
            operation_id: "process-resource-watch".to_owned(),
            idempotency_key: None,
            correlation_id: "process-resource-watch".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(PROCESS_TYPE).expect("static Process type"),
            ResourceTypeName::parse(EPHEMERAL_PROCESS_TYPE).expect("static EphemeralProcess type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        after_revision: ZoneRevision::new(0),
        initial_credits: 64,
        projection: StoreProjection::Full,
    }
}

/// Relist all generic Process resources, preserving snapshot pagination.
pub(crate) async fn list_process_snapshot(
    store: &RedbResourceStore,
    zone: &ZoneId,
) -> Result<Vec<StoredResource>, ProcessResourceRuntimeError> {
    let mut request = process_list_request(zone);
    let mut resources = Vec::new();
    loop {
        let result = store
            .list(request.clone())
            .await
            .map_err(|_| ProcessResourceRuntimeError::Store)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

/// Relist generic Process resources through a session-bound Resource API
/// backend. This mirrors the concrete Zone-store helper while preserving the
/// backend's reconnect fence.
pub(crate) async fn list_process_snapshot_backend<S: ResourceStoreBackend>(
    store: &S,
    zone: &ZoneId,
) -> Result<Vec<StoredResource>, ProcessResourceRuntimeError> {
    let mut request = process_list_request(zone);
    let mut resources = Vec::new();
    loop {
        let result = store
            .list(request.clone())
            .await
            .map_err(|_| ProcessResourceRuntimeError::Store)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

/// Run Guest-local Process/EphemeralProcess reconciliation for one
/// authenticated ComponentSession. The session-bound store is intentionally
/// relisted instead of opening a second transport or watch implementation;
/// reconnect fencing makes the loop stop at the first stale-session error.
pub(crate) async fn run_guest_process_reconciliation<S>(
    mut runtime: ProcessResourceRuntime,
    store: Arc<S>,
    client: Arc<ResourceApiClient<S, UnavailableUpgradeDispatcher>>,
    zone: ZoneId,
) where
    S: ResourceStoreBackend + 'static,
{
    runtime.set_status_client(client);
    loop {
        let snapshot = match list_process_snapshot_backend(store.as_ref(), &zone).await {
            Ok(snapshot) => snapshot,
            Err(_) => return,
        };
        if let Err(error) = runtime.reconcile(snapshot).await {
            tracing::warn!(zone = %zone.as_str(), error = %error, "Guest Process reconciliation degraded");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Drain a deleted WaylandSession through its durable Process and Endpoint
/// children before releasing the session finalizer.
pub(crate) async fn reconcile_wayland_session_deletion(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    store: &RedbResourceStore,
    zone: &ZoneId,
    session_ref: &ResourceRef,
) -> Result<(), ProcessResourceRuntimeError> {
    for _ in 0..8 {
        let session = match store
            .get(StoreGetRequest {
                operation: cleanup_operation("wayland-session-get", session_ref, 0),
                zone: zone.clone(),
                target: session_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
        {
            Ok(resource) => resource,
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => return Ok(()),
            Err(_) => return Err(ProcessResourceRuntimeError::Store),
        };
        if session.resource_ref.resource_type().as_str() != WAYLAND_SESSION_TYPE
            || !metadata_deletion_requested(&session)
        {
            return Ok(());
        }

        let children = list_cleanup_children(store, zone).await?;
        let owned_processes = children
            .iter()
            .filter(|resource| {
                resource.resource_ref.resource_type().as_str() == PROCESS_TYPE
                    && metadata_owner_ref(resource).as_ref() == Some(session_ref)
            })
            .cloned()
            .collect::<Vec<_>>();
        let owned_endpoints = children
            .iter()
            .filter(|resource| {
                resource.resource_ref.resource_type().as_str() == "Endpoint"
                    && metadata_owner_ref(resource).as_ref() == Some(session_ref)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut changed = false;

        for process in &owned_processes {
            if !metadata_deletion_requested(process) {
                changed |=
                    request_cleanup_delete(client, process, "wayland-child-process-delete").await;
            } else if matches!(
                status_phase(process),
                Some(ResourcePhase::Succeeded | ResourcePhase::Failed | ResourcePhase::Deleted)
            ) && !owned_endpoints.iter().any(|endpoint| {
                endpoint_producer_ref(endpoint).as_ref() == Some(&process.resource_ref)
            }) {
                changed |=
                    request_cleanup_delete(client, process, "wayland-child-process-drain").await;
            }
        }

        for endpoint in &owned_endpoints {
            let producer = endpoint_producer_ref(endpoint);
            let producer_terminal = producer.as_ref().is_none_or(|producer| {
                owned_processes
                    .iter()
                    .find(|process| &process.resource_ref == producer)
                    .is_none_or(|process| {
                        matches!(
                            status_phase(process),
                            Some(
                                ResourcePhase::Succeeded
                                    | ResourcePhase::Failed
                                    | ResourcePhase::Deleted
                            )
                        )
                    })
            });
            if producer_terminal && !metadata_deletion_requested(endpoint) {
                changed |=
                    request_cleanup_delete(client, endpoint, "wayland-child-endpoint-delete")
                        .await;
            } else if producer_terminal && metadata_deletion_requested(endpoint) {
                changed |=
                    request_cleanup_delete(client, endpoint, "wayland-child-endpoint-drain").await;
            }
        }

        let refreshed_children = list_cleanup_children(store, zone).await?;
        let remaining = refreshed_children.iter().any(|resource| {
            matches!(
                resource.resource_ref.resource_type().as_str(),
                PROCESS_TYPE | "Endpoint"
            ) && metadata_owner_ref(resource).as_ref() == Some(session_ref)
        });
        if !remaining {
            let current = store
                .get(StoreGetRequest {
                    operation: cleanup_operation("wayland-session-finalizer-get", session_ref, 0),
                    zone: zone.clone(),
                    target: session_ref.clone(),
                    expected_uid: None,
                    projection: StoreProjection::Full,
                })
                .await
                .map_err(|_| ProcessResourceRuntimeError::Store)?;
            if metadata_has_finalizer(&current, WAYLAND_SESSION_FINALIZER) {
                changed |= request_cleanup_finalizer(
                    client,
                    &current,
                    WAYLAND_SESSION_FINALIZER,
                    false,
                )
                .await;
            } else {
                changed |=
                    request_cleanup_delete(client, &current, "wayland-session-delete").await;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(())
}

async fn list_cleanup_children(
    store: &RedbResourceStore,
    zone: &ZoneId,
) -> Result<Vec<StoredResource>, ProcessResourceRuntimeError> {
    let mut request = StoreListRequest {
        operation: StoreOperationContext {
            operation_id: "wayland-session-cleanup-list".to_owned(),
            idempotency_key: None,
            correlation_id: "wayland-session-cleanup-list".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: vec![
            ResourceTypeName::parse(PROCESS_TYPE).expect("static Process type"),
            ResourceTypeName::parse("Endpoint").expect("static Endpoint type"),
        ],
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 256,
        cursor: None,
        projection: StoreProjection::Full,
    };
    let mut resources = Vec::new();
    loop {
        let result = store
            .list(request.clone())
            .await
            .map_err(|_| ProcessResourceRuntimeError::Store)?;
        resources.extend(result.resources);
        let Some(cursor) = result.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

fn cleanup_operation(action: &str, resource_ref: &ResourceRef, revision: u64) -> StoreOperationContext {
    let operation_id = cleanup_operation_id(action, resource_ref, revision);
    StoreOperationContext {
        operation_id: operation_id.clone(),
        idempotency_key: None,
        correlation_id: operation_id,
        trace_id: None,
        deadline_ms: 10_000,
    }
}

fn cleanup_operation_id(action: &str, resource_ref: &ResourceRef, revision: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(action.as_bytes());
    digest.update([0]);
    digest.update(resource_ref.to_canonical_string().as_bytes());
    digest.update(revision.to_be_bytes());
    let digest = digest.finalize();
    let suffix = digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{action}-{suffix}")
}

fn metadata_deletion_requested(resource: &StoredResource) -> bool {
    metadata_value(resource, "deletionRequestedAt")
        .is_some_and(|value| !matches!(value, CanonicalJsonValue::Null))
}

fn metadata_has_finalizer(resource: &StoredResource, expected: &str) -> bool {
    metadata_value(resource, "finalizers").is_some_and(|value| {
        matches!(
            value,
            CanonicalJsonValue::Array(values)
                if values.iter().any(|value| {
                    matches!(value, CanonicalJsonValue::String(value) if value == expected)
                })
        )
    })
}

fn metadata_owner_ref(resource: &StoredResource) -> Option<ResourceRef> {
    let CanonicalJsonValue::String(value) = metadata_value(resource, "ownerRef")? else {
        return None;
    };
    ResourceRef::parse(&value).ok()
}

fn endpoint_producer_ref(resource: &StoredResource) -> Option<ResourceRef> {
    let value = CanonicalJsonValue::parse(&resource.canonical_json).ok()?;
    let CanonicalJsonValue::Object(root) = value else {
        return None;
    };
    let CanonicalJsonValue::Object(spec) = root.get("spec")? else {
        return None;
    };
    let CanonicalJsonValue::String(value) = spec.get("producerRef")? else {
        return None;
    };
    ResourceRef::parse(&value).ok()
}

fn cleanup_wire_identity(resource: &StoredResource) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = resource.zone.as_str().to_owned();
    identity.resource_type = resource.resource_ref.resource_type().as_str().to_owned();
    identity.name = resource.resource_ref.name().as_str().to_owned();
    identity.uid = Some(resource.uid.as_str().to_owned());
    identity.generation = Some(resource.generation.get());
    identity.revision = Some(resource.revision.get());
    identity
}

async fn request_cleanup_delete(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    resource: &StoredResource,
    action: &str,
) -> bool {
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = protobuf::MessageField::some(cleanup_wire_identity(resource));
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(resource.revision.get());
    precondition.expected_uid = Some(resource.uid.as_str().to_owned());
    mutation.precondition = protobuf::MessageField::some(precondition);
    let mut request = wire::DeleteRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(
        &cleanup_operation_id(action, &resource.resource_ref, resource.revision.get()),
    ));
    request.mutation = protobuf::MessageField::some(mutation);
    client.delete(request).await.error.is_none()
}

async fn request_cleanup_finalizer(
    client: &ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>,
    resource: &StoredResource,
    finalizer: &str,
    add: bool,
) -> bool {
    let mut mutation = wire::Mutation::new();
    mutation.kind =
        protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS);
    mutation.target = protobuf::MessageField::some(cleanup_wire_identity(resource));
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_revision = Some(resource.revision.get());
    precondition.expected_uid = Some(resource.uid.as_str().to_owned());
    mutation.precondition = protobuf::MessageField::some(precondition);
    if add {
        mutation.add_finalizers.push(finalizer.to_owned());
    } else {
        mutation.remove_finalizers.push(finalizer.to_owned());
    }
    let mut request = wire::UpdateFinalizersRequest::new();
    let action = if add {
        "wayland-session-finalizer-add"
    } else {
        "wayland-session-finalizer-remove"
    };
    request.meta = protobuf::MessageField::some(request_meta(
        &cleanup_operation_id(action, &resource.resource_ref, resource.revision.get()),
    ));
    request.mutation = protobuf::MessageField::some(mutation);
    client.update_finalizers(request).await.error.is_none()
}

/// Run the relist/watch reconciliation loop for one Zone.
pub(crate) async fn run_process_watch(
    mut watch: ResourceWatch,
    store: Arc<RedbResourceStore>,
    zone: ZoneId,
    registry: Arc<Mutex<Option<ProcessResourceRuntime>>>,
    status_client: Option<Arc<ResourceApiClient<RedbBackend, UnavailableUpgradeDispatcher>>>,
    wayland_session_ref: Option<ResourceRef>,
) {
    loop {
        match tokio::time::timeout(Duration::from_secs(1), watch.recv()).await {
            Ok(Some(batch)) => {
                if watch.acknowledge(batch.revision()).await.is_err() {
                    return;
                }
            }
            Ok(None) => {
                if watch.resume().await.is_err() {
                    return;
                }
            }
            Err(_) => {}
        }
        let Ok(snapshot) = list_process_snapshot(&store, &zone).await else {
            continue;
        };
        let runtime = match registry.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => return,
        };
        let Some(mut runtime) = runtime else {
            continue;
        };
        let result = runtime.reconcile(snapshot).await;
        if let Ok(mut guard) = registry.lock() {
            *guard = Some(runtime);
        } else {
            return;
        }
        if result.is_err() {
            tracing::warn!(zone = %zone.as_str(), "generic Process reconciliation degraded");
        }
        if let (Some(client), Some(session_ref)) =
            (status_client.as_ref(), wayland_session_ref.as_ref())
            && let Err(error) =
                reconcile_wayland_session_deletion(client, &store, &zone, session_ref).await
        {
            tracing::warn!(
                zone = %zone.as_str(),
                error = %error,
                "WaylandSession deletion drain degraded"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_requests_use_both_generic_resource_types() {
        let zone = ZoneId::parse("test").expect("valid zone");
        let request = process_list_request(&zone);
        assert_eq!(request.resource_types.len(), 2);
        assert_eq!(request.resource_types[0].as_str(), PROCESS_TYPE);
        assert_eq!(request.resource_types[1].as_str(), EPHEMERAL_PROCESS_TYPE);
    }

    #[test]
    fn unsupported_provider_is_rejected_before_lifecycle_effects() {
        let provider = ResourceRef::parse("Provider/audio-pipewire").expect("valid provider");
        assert_eq!(
            provider.name().as_str(),
            "audio-pipewire",
            "the decoder keeps Provider identity opaque until the fixed allow-list"
        );
    }

    #[test]
    fn deletion_retains_finalizer_when_guest_execution_is_unavailable() {
        assert!(matches!(
            deletion_adoption(Err(GUEST_EXECUTION_UNAVAILABLE.to_owned())),
            Err(ProcessResourceRuntimeError::ProviderEffect)
        ));
        assert!(matches!(
            deletion_adoption(Err("provider-ticket:other".to_owned())),
            Err(ProcessResourceRuntimeError::ProviderEffect)
        ));
    }

    #[test]
    fn status_projection_keeps_the_complete_envelope_valid() {
        let resource_ref = ResourceRef::parse("Process/status-projection").expect("resource ref");
        let process = serde_json::from_str::<ProcessSpec>(
            r#"{"executionRef":"Host/host-system","processClass":"worker","template":"reaction"}"#,
        )
        .expect("minimal Process spec");
        let resource = StoredResource {
            resource_ref: resource_ref.clone(),
            zone: ZoneId::parse("dev").expect("zone"),
            uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .expect("uid"),
            generation: d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("generation"),
            revision: ZoneRevision::new(1),
            canonical_json: br#"{"apiVersion":"resources.d2bus.org/v3","metadata":{"configurationGeneration":1,"createdAt":"2026-07-22T00:00:00.000Z","deletionRequestedAt":null,"finalizers":[],"generation":1,"managedBy":"configuration","name":"status-projection","ownerRef":null,"revision":1,"uid":"123e4567-e89b-42d3-a456-426614174000","updatedAt":"2026-07-22T00:00:00.000Z","zone":"dev"},"spec":{"executionRef":"Host/host-system","processClass":"worker","template":"reaction"},"status":{"completedAt":null,"conditions":[],"lastReconciledAt":null,"observedGeneration":0,"outcome":null,"phase":"Pending","resource":{},"startedAt":null,"update":{"dependencies":{"count":0,"refs":[]},"disruption":"None","lastAssessedAt":null,"observedGeneration":0,"operationId":null,"owned":{"count":0,"refs":[]},"preserveState":true,"reasons":[],"state":"Unknown","targetGeneration":1}},"type":"Process"}"#.to_vec(),
            payload_digest: "sha256:".to_owned(),
        };
        let record = DesiredRecord {
            resource,
            provider_ref: ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
            process: DesiredProcess::Process(process),
        };
        let canonical = status_payload(
            &record,
            ResourcePhase::Ready,
            2,
            Some(OutcomeState::ready(false)),
        )
        .expect("status payload");
        let envelope = ResourceEnvelope::from_json(&canonical).expect("valid envelope");
        assert_eq!(envelope.status().phase(), ResourcePhase::Ready);
        assert_eq!(
            envelope
                .status()
                .resource()
                .get("restartCount")
                .and_then(|value| match value {
                    CanonicalJsonValue::Integer(value) => Some(*value),
                    _ => None,
                }),
            Some(2)
        );
        assert!(d2b_contracts_resource::v3::Timestamp::parse(now_timestamp()).is_ok());
        assert_eq!(record.key(), resource_ref);
    }

    #[test]
    fn lifecycle_effects_use_contract_deadlines() {
        let ephemeral = serde_json::from_str::<EphemeralProcessSpec>(
            r#"{"executionRef":"Host/host-system","processClass":"worker","template":"reaction","startDeadline":"7s","runtimeDeadline":"5m","successfulTtl":"1h","failedTtl":"24h","incidentHold":false}"#,
        )
        .expect("ephemeral process spec");
        assert_eq!(
            launch_timeout(&DesiredProcess::Ephemeral(ephemeral)),
            Duration::from_secs(7)
        );

        let process = serde_json::from_str::<ProcessSpec>(
            r#"{"executionRef":"Host/host-system","processClass":"controller","template":"controller-main","drainTimeout":"11s"}"#,
        )
        .expect("process spec");
        assert_eq!(process_drain_timeout(&process), Duration::from_secs(11));
    }

    #[test]
    fn guest_target_selector_is_limited_to_the_wayland_session_owner() {
        let session_ref = ResourceRef::parse(
            "display-wayland.d2bus.org.WaylandSession/display-wayland",
        )
        .expect("session ref");
        let guest_ref = ResourceRef::parse("Guest/work").expect("guest ref");
        let provider_ref = ResourceRef::parse("Provider/system-minijail").expect("provider ref");
        let process = serde_json::from_str::<ProcessSpec>(
            r#"{"executionRef":"Host/host-system","processClass":"worker","template":"reaction"}"#,
        )
        .expect("process");
        let make_record = |owner_ref: &str| DesiredRecord {
            resource: StoredResource {
                resource_ref: ResourceRef::parse("Process/worker").expect("process ref"),
                zone: ZoneId::parse("work").expect("zone"),
                uid: d2b_contracts_resource::v3::ResourceUid::parse(
                    "123e4567-e89b-42d3-a456-426614174000",
                )
                .expect("uid"),
                generation: d2b_contracts_resource::v3::ResourceGeneration::new(1)
                    .expect("generation"),
                revision: ZoneRevision::new(1),
                canonical_json: format!(r#"{{"metadata":{{"ownerRef":"{owner_ref}"}}}}"#)
                    .into_bytes(),
                payload_digest: "sha256:".to_owned(),
            },
            provider_ref: provider_ref.clone(),
            process: DesiredProcess::Process(process.clone()),
        };

        assert_eq!(
            scoped_target_ref(
                &make_record(session_ref.to_canonical_string().as_str()),
                Some(&session_ref),
                Some(&guest_ref),
            ),
            Some(guest_ref.clone())
        );
        assert_eq!(
            scoped_target_ref(
                &make_record("Provider/other"),
                Some(&session_ref),
                Some(&guest_ref),
            ),
            None
        );
    }
}
