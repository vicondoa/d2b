//! ACA Guest lifecycle controller.

use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::time::{Duration, Instant, timeout_at};

use d2b_contracts_resource::v3::ResourceRef;

use crate::{
    AcaControl, AcaControlContext, AcaControlError, AcaControlErrorKind, AcaControlHealth,
    AcaCredentialLease, AcaCredentialLeaseClient, AcaCredentialLeaseRequest, AcaCredentialPurpose,
    AcaDesiredDiskImage, AcaDesiredSandbox, AcaDiskImageRecord, AcaOperationId, AcaProviderConfig,
    AcaResourceBinding, AcaRuntimeConfig, AcaSandboxCandidates, AcaSandboxLifecycle,
    AcaSandboxRecord, AcaTypeError, AcaWorkloadQuery,
};

/// Provider lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AcaPhase {
    /// No remote sandbox has been observed.
    Pending,
    /// A remote sandbox or disk image is being provisioned.
    Provisioning,
    /// A remote sandbox is being started.
    Starting,
    /// The sandbox and its authenticated Endpoint are ready.
    Ready,
    /// A transient or dependency failure can be retried.
    Degraded,
    /// The current generation failed closed.
    Failed,
    /// Finalization is stopping and deleting the remote sandbox.
    Finalizing,
    /// Finalization completed.
    Finalized,
}

/// Default descriptor repair interval.
pub const ACA_REPAIR_INTERVAL_SECS: u64 = 30;
/// Exact Guest finalizer owned by the ACA runtime Provider.
pub const ACA_GUEST_FINALIZER: &str = "runtime-azure-container-apps.d2bus.org/guest-cleanup";

/// Result of one non-blocking reconcile pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaReconcileOutcome {
    /// The desired lifecycle is converged.
    Converged,
    /// A bounded retry should be scheduled.
    Retry {
        /// Retry delay in milliseconds.
        after_ms: u32,
    },
    /// An asynchronous provider operation is still progressing.
    Progressing {
        /// Poll delay in milliseconds.
        after_ms: u32,
    },
}

/// Controller failures with stable, bounded diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcaControllerError {
    /// The state machine was called after finalization.
    InvalidState,
    /// More than one matching sandbox was found.
    AmbiguousAdoption,
    /// No sandbox was available for a requested operation.
    SandboxUnavailable,
    /// The injected effect failed.
    Effect(AcaControlErrorKind),
    /// Credential cleanup failed after an otherwise successful operation.
    LeaseCleanup(AcaControlErrorKind),
    /// The sandbox did not become ready within the configured attempt bound.
    ReadinessExhausted,
}

impl AcaControllerError {
    /// Return the stable public error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidState => "aca-invalid-state",
            Self::AmbiguousAdoption => "aca-ambiguous-adoption",
            Self::SandboxUnavailable => "aca-sandbox-unavailable",
            Self::Effect(kind) | Self::LeaseCleanup(kind) => kind.code(),
            Self::ReadinessExhausted => "aca-readiness-exhausted",
        }
    }
}

impl fmt::Display for AcaControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AcaControllerError {}

/// Small bounded completed-operation ledger used by the controller adapter.
#[derive(Debug, Default)]
pub struct CompletedOperationLedger {
    completed: BTreeMap<AcaOperationId, (u64, AcaPhase, u64)>,
    next_sequence: u64,
}

/// Clock used to turn bounded operation deadlines into absolute Unix expiry.
pub trait AcaClock: Send + Sync {
    /// Return the current Unix time in milliseconds.
    fn now_unix_ms(&self) -> u64;
}

/// Production wall clock for ACA lease expiry.
#[derive(Debug, Default)]
pub struct SystemAcaClock;

impl AcaClock for SystemAcaClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AcaFinalizationStage {
    Observe,
    Stop,
    Delete,
}

impl CompletedOperationLedger {
    /// Record one operation and evict the oldest entries at capacity.
    pub fn record(
        &mut self,
        operation_id: AcaOperationId,
        expires_at_unix_ms: u64,
        phase: AcaPhase,
        capacity: usize,
    ) {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.completed
            .insert(operation_id, (expires_at_unix_ms, phase, sequence));
        while self.completed.len() > capacity {
            let Some(oldest) = self
                .completed
                .iter()
                .min_by_key(|(_, (_, _, sequence))| *sequence)
                .map(|(operation_id, _)| operation_id.clone())
            else {
                break;
            };
            self.completed.remove(&oldest);
        }
    }

    /// Remove expired operation records.
    pub fn prune(&mut self, now_unix_ms: u64) {
        self.completed
            .retain(|_, (expires_at, _, _)| *expires_at > now_unix_ms);
    }

    /// Return a previously completed phase.
    pub fn get(&self, operation_id: &AcaOperationId) -> Option<AcaPhase> {
        self.completed.get(operation_id).map(|(_, phase, _)| *phase)
    }
}

/// Canonical ACA lifecycle controller.
pub struct AcaController<C, L> {
    binding: AcaResourceBinding,
    config: AcaRuntimeConfig,
    network_ref: Option<ResourceRef>,
    sandbox_transport_alias: crate::AcaProfileId,
    control: Arc<C>,
    leases: Arc<L>,
    phase: AcaPhase,
    finalizer: bool,
    observed: Option<AcaSandboxRecord>,
    ledger: CompletedOperationLedger,
    clock: Arc<dyn AcaClock>,
    readiness_generation: u64,
    readiness_attempts: u8,
    readiness_lifecycle: Option<AcaSandboxLifecycle>,
    finalization_stage: AcaFinalizationStage,
}

impl<C, L> AcaController<C, L>
where
    C: AcaControl + 'static,
    L: AcaCredentialLeaseClient + 'static,
{
    /// Construct a controller for one Guest binding.
    pub fn new(
        binding: AcaResourceBinding,
        config: AcaRuntimeConfig,
        control: Arc<C>,
        leases: Arc<L>,
    ) -> Self {
        let generation = binding.provider_generation;
        let sandbox_transport_alias = config.profile().profile_id().clone();
        Self {
            binding,
            config,
            network_ref: None,
            sandbox_transport_alias,
            control,
            leases,
            phase: AcaPhase::Pending,
            finalizer: true,
            observed: None,
            ledger: CompletedOperationLedger::default(),
            clock: Arc::new(SystemAcaClock),
            readiness_generation: generation,
            readiness_attempts: 0,
            readiness_lifecycle: None,
            finalization_stage: AcaFinalizationStage::Observe,
        }
    }

    /// Replace the wall clock used for lease expiry and operation retention.
    pub fn with_clock(mut self, clock: Arc<dyn AcaClock>) -> Self {
        self.clock = clock;
        self
    }

    /// Bind provider-level network and sandbox transport settings to effects.
    pub fn with_provider_settings(
        mut self,
        network_ref: Option<ResourceRef>,
        sandbox_transport_alias: crate::AcaProfileId,
    ) -> Self {
        self.network_ref = network_ref;
        self.sandbox_transport_alias = sandbox_transport_alias;
        self
    }

    /// Return the current phase.
    pub const fn phase(&self) -> AcaPhase {
        self.phase
    }

    /// Return whether the finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Reconcile using external observation before any ensure effect.
    pub async fn reconcile(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        self.ensure_active()?;
        self.ledger.prune(self.clock.now_unix_ms());
        if let Some(outcome) = self.completed_outcome(&operation_id) {
            return Ok(outcome);
        }
        let query = AcaWorkloadQuery {
            binding: self.binding.clone(),
            profile_id: self.config.profile().profile_id().clone(),
        };
        let candidates = self
            .with_lease(
                operation_id.clone(),
                AcaCredentialPurpose::Inspect,
                deadline_remaining_ms,
                move |control, lease, context| async move {
                    control.find_sandboxes(&lease, &context, &query).await
                },
            )
            .await?;
        let candidate = match one_candidate(candidates) {
            Ok(candidate) => candidate,
            Err(error) => {
                tracing::warn!(
                    resource = %self.binding.guest_uid,
                    provider = "runtime-azure-container-apps",
                    code = error.code(),
                    "sandbox candidate resolution failed during reconcile"
                );
                self.phase = AcaPhase::Degraded;
                return Err(error);
            }
        };
        match candidate {
            Some(record) => {
                self.ensure_sandbox_generation(&record)?;
                self.reconcile_observed(operation_id, deadline_remaining_ms, record)
                    .await
            }
            None => {
                self.ensure_sandbox(operation_id, deadline_remaining_ms)
                    .await
            }
        }
    }

    /// Finalize child-first by stopping before deleting the remote sandbox.
    pub async fn finalize(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
    ) -> Result<(), AcaControllerError> {
        if !self.finalizer {
            return Ok(());
        }
        self.phase = AcaPhase::Finalizing;
        if self.finalization_stage == AcaFinalizationStage::Observe || self.observed.is_none() {
            let query = AcaWorkloadQuery {
                binding: self.binding.clone(),
                profile_id: self.config.profile().profile_id().clone(),
            };
            let candidates = self
                .with_lease(
                    operation_id.clone(),
                    AcaCredentialPurpose::Destroy,
                    deadline_remaining_ms,
                    move |control, lease, context| async move {
                        control.find_sandboxes(&lease, &context, &query).await
                    },
                )
                .await?;
            self.observed = one_candidate(candidates)?;
            if let Some(record) = self.observed.as_ref() {
                self.ensure_sandbox_generation(record)?;
            }
            if self.observed.is_none() {
                self.finish_finalization();
                return Ok(());
            }
        }
        if self.finalization_stage == AcaFinalizationStage::Stop
            && !self.observed.as_ref().is_some_and(|record| {
                matches!(
                    record.lifecycle,
                    AcaSandboxLifecycle::Stopped | AcaSandboxLifecycle::Failed
                )
            })
        {
            let query = AcaWorkloadQuery {
                binding: self.binding.clone(),
                profile_id: self.config.profile().profile_id().clone(),
            };
            let candidates = self
                .with_lease(
                    operation_id.clone(),
                    AcaCredentialPurpose::Destroy,
                    deadline_remaining_ms,
                    move |control, lease, context| async move {
                        control.find_sandboxes(&lease, &context, &query).await
                    },
                )
                .await?;
            self.observed = one_candidate(candidates)?;
            let Some(record) = self.observed.as_ref() else {
                self.finish_finalization();
                return Ok(());
            };
            self.ensure_sandbox_generation(record)?;
            match record.lifecycle {
                AcaSandboxLifecycle::Creating | AcaSandboxLifecycle::Stopping => return Ok(()),
                AcaSandboxLifecycle::Stopped => {
                    self.finalization_stage = AcaFinalizationStage::Delete;
                }
                AcaSandboxLifecycle::Running | AcaSandboxLifecycle::Suspended => {}
                AcaSandboxLifecycle::Failed => {
                    self.finalization_stage = AcaFinalizationStage::Delete;
                }
                AcaSandboxLifecycle::Unknown => {
                    tracing::warn!(
                        resource = %self.binding.guest_uid,
                        provider = "runtime-azure-container-apps",
                        "finalization refused: sandbox lifecycle unknown during stop stage"
                    );
                    self.phase = AcaPhase::Degraded;
                    return Err(AcaControllerError::Effect(AcaControlErrorKind::Ambiguous));
                }
            }
        }
        if self.finalization_stage == AcaFinalizationStage::Observe {
            self.finalization_stage = match self.observed.as_ref().map(|record| record.lifecycle) {
                Some(AcaSandboxLifecycle::Running | AcaSandboxLifecycle::Suspended) => {
                    AcaFinalizationStage::Stop
                }
                Some(AcaSandboxLifecycle::Stopped) => AcaFinalizationStage::Delete,
                Some(AcaSandboxLifecycle::Creating | AcaSandboxLifecycle::Stopping) => {
                    self.finalization_stage = AcaFinalizationStage::Stop;
                    return Ok(());
                }
                Some(AcaSandboxLifecycle::Failed) => AcaFinalizationStage::Delete,
                Some(AcaSandboxLifecycle::Unknown) => {
                    tracing::warn!(
                        resource = %self.binding.guest_uid,
                        provider = "runtime-azure-container-apps",
                        "finalization refused: sandbox lifecycle unknown during observe stage"
                    );
                    self.phase = AcaPhase::Degraded;
                    return Err(AcaControllerError::Effect(AcaControlErrorKind::Ambiguous));
                }
                None => {
                    self.finish_finalization();
                    return Ok(());
                }
            };
        }
        if self.finalization_stage == AcaFinalizationStage::Stop {
            let record = self
                .observed
                .clone()
                .ok_or(AcaControllerError::SandboxUnavailable)?;
            let stopped = self
                .with_lease(
                    operation_id.clone(),
                    AcaCredentialPurpose::Stop,
                    deadline_remaining_ms,
                    move |control, lease, context| async move {
                        control.stop_sandbox(&lease, &context, &record.id).await
                    },
                )
                .await?;
            self.observed = Some(stopped);
            match self.observed.as_ref().map(|record| record.lifecycle) {
                Some(AcaSandboxLifecycle::Stopped) => {
                    self.finalization_stage = AcaFinalizationStage::Delete;
                }
                Some(
                    AcaSandboxLifecycle::Creating
                    | AcaSandboxLifecycle::Stopping
                    | AcaSandboxLifecycle::Running
                    | AcaSandboxLifecycle::Suspended,
                ) => return Ok(()),
                Some(AcaSandboxLifecycle::Failed) => {
                    self.finalization_stage = AcaFinalizationStage::Delete;
                }
                Some(AcaSandboxLifecycle::Unknown) => {
                    tracing::warn!(
                        resource = %self.binding.guest_uid,
                        provider = "runtime-azure-container-apps",
                        "finalization refused: sandbox lifecycle unknown after stop"
                    );
                    self.phase = AcaPhase::Degraded;
                    return Err(AcaControllerError::Effect(AcaControlErrorKind::Ambiguous));
                }
                None => {
                    self.finish_finalization();
                    return Ok(());
                }
            }
        }
        if self.finalization_stage == AcaFinalizationStage::Delete {
            if self
                .observed
                .as_ref()
                .is_some_and(|record| record.lifecycle == AcaSandboxLifecycle::Stopping)
            {
                self.finalization_stage = AcaFinalizationStage::Stop;
                return Ok(());
            }
            let record = self
                .observed
                .clone()
                .ok_or(AcaControllerError::SandboxUnavailable)?;
            if record.lifecycle == AcaSandboxLifecycle::Stopping {
                return Ok(());
            }
            let outcome = self
                .with_lease(
                    operation_id,
                    AcaCredentialPurpose::Destroy,
                    deadline_remaining_ms,
                    move |control, lease, context| async move {
                        control.delete_sandbox(&lease, &context, &record.id).await
                    },
                )
                .await?;
            match outcome {
                crate::AcaDeleteOutcome::Deleted | crate::AcaDeleteOutcome::AlreadyAbsent => {
                    self.finish_finalization();
                }
            }
        }
        Ok(())
    }

    async fn reconcile_observed(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
        record: AcaSandboxRecord,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        let lifecycle = record.lifecycle;
        self.observed = Some(record);
        match lifecycle {
            AcaSandboxLifecycle::Running => {
                match self
                    .health(operation_id.clone(), deadline_remaining_ms)
                    .await
                {
                    Ok(AcaControlHealth::Ready) => {
                        self.reset_readiness();
                        self.phase = AcaPhase::Ready;
                        self.record(operation_id);
                        Ok(AcaReconcileOutcome::Converged)
                    }
                    Ok(AcaControlHealth::Degraded | AcaControlHealth::Unavailable) => {
                        self.readiness_retry(AcaSandboxLifecycle::Running)
                    }
                    Err(error) if self.retryable_error(error) => {
                        self.readiness_retry(AcaSandboxLifecycle::Running)
                    }
                    Err(error) => {
                        self.phase = AcaPhase::Degraded;
                        Err(error)
                    }
                }
            }
            AcaSandboxLifecycle::Suspended | AcaSandboxLifecycle::Stopped => {
                self.phase = AcaPhase::Starting;
                let id = self.observed.take().expect("stored above").id;
                let resumed = self
                    .with_lease(
                        operation_id.clone(),
                        AcaCredentialPurpose::Start,
                        deadline_remaining_ms,
                        move |control, lease, context| async move {
                            control.resume_sandbox(&lease, &context, &id).await
                        },
                    )
                    .await?;
                self.observed = Some(resumed);
                let resumed = self.observed.as_ref().expect("stored above");
                if resumed.lifecycle == AcaSandboxLifecycle::Running {
                    match self
                        .health(operation_id.clone(), deadline_remaining_ms)
                        .await
                    {
                        Ok(AcaControlHealth::Ready) => {
                            self.reset_readiness();
                            self.phase = AcaPhase::Ready;
                            self.record(operation_id);
                            Ok(AcaReconcileOutcome::Converged)
                        }
                        Ok(AcaControlHealth::Degraded | AcaControlHealth::Unavailable) => {
                            self.readiness_retry(AcaSandboxLifecycle::Running)
                        }
                        Err(error) if self.retryable_error(error) => {
                            self.readiness_retry(AcaSandboxLifecycle::Running)
                        }
                        Err(error) => {
                            self.phase = AcaPhase::Degraded;
                            Err(error)
                        }
                    }
                } else {
                    self.readiness_retry(resumed.lifecycle)
                }
            }
            AcaSandboxLifecycle::Creating | AcaSandboxLifecycle::Stopping => {
                self.readiness_retry(lifecycle)
            }
            AcaSandboxLifecycle::Failed | AcaSandboxLifecycle::Unknown => {
                self.readiness_retry(lifecycle)
            }
        }
    }

    async fn ensure_sandbox(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        self.phase = AcaPhase::Provisioning;
        let desired_disk = AcaDesiredDiskImage {
            source: self.config.profile().disk_image().clone(),
        };
        let generation = self.binding.provider_generation;
        let image = self
            .with_lease(
                operation_id.clone(),
                AcaCredentialPurpose::Ensure,
                deadline_remaining_ms,
                move |control, lease, context| async move {
                    let candidates = control
                        .find_disk_images(&lease, &context, &desired_disk)
                        .await?;
                    if let Some(record) = one_disk_image(candidates, generation)? {
                        Ok(record)
                    } else {
                        let record = control
                            .create_disk_image(&lease, &context, &desired_disk)
                            .await?;
                        if record.generation != generation {
                            return Err(AcaControlError::new(AcaControlErrorKind::Conflict));
                        }
                        Ok(record)
                    }
                },
            )
            .await?;
        let desired = AcaDesiredSandbox {
            binding: self.binding.clone(),
            profile: self.config.profile().clone(),
            disk_image: image,
            network_ref: self.network_ref.clone(),
            sandbox_transport_alias: self.sandbox_transport_alias.clone(),
        };
        let created = self
            .with_lease(
                operation_id.clone(),
                AcaCredentialPurpose::Ensure,
                deadline_remaining_ms,
                move |control, lease, context| async move {
                    control.create_sandbox(&lease, &context, &desired).await
                },
            )
            .await?;
        self.observed = Some(created);
        self.readiness_generation = self.binding.provider_generation;
        self.readiness_attempts = 0;
        self.readiness_lifecycle = None;
        Ok(AcaReconcileOutcome::Progressing {
            after_ms: self.config.readiness().interval_ms(),
        })
    }

    async fn health(
        &self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
    ) -> Result<AcaControlHealth, AcaControllerError> {
        self.with_lease(
            operation_id,
            AcaCredentialPurpose::Health,
            deadline_remaining_ms,
            move |control, lease, context| async move { control.health(&lease, &context).await },
        )
        .await
    }

    async fn with_lease<T, F, Fut>(
        &self,
        operation_id: AcaOperationId,
        purpose: AcaCredentialPurpose,
        deadline_remaining_ms: u32,
        call: F,
    ) -> Result<T, AcaControllerError>
    where
        F: FnOnce(Arc<C>, AcaCredentialLease, AcaControlContext) -> Fut,
        Fut: std::future::Future<Output = Result<T, AcaControlError>>,
    {
        if deadline_remaining_ms == 0 {
            tracing::warn!(
                resource = %self.binding.guest_uid,
                provider = "runtime-azure-container-apps",
                "operation deadline already expired before credential lease acquisition"
            );
            return Err(AcaControllerError::Effect(
                AcaControlErrorKind::DeadlineExpired,
            ));
        }
        let request = AcaCredentialLeaseRequest::new(
            operation_id.clone(),
            purpose,
            self.clock
                .now_unix_ms()
                .saturating_add(u64::from(deadline_remaining_ms)),
        );
        let deadline = Instant::now() + Duration::from_millis(u64::from(deadline_remaining_ms));
        let lease = timeout_at(deadline, self.leases.acquire(&request))
            .await
            .map_err(|_| {
                tracing::warn!(
                    resource = %self.binding.guest_uid,
                    provider = "runtime-azure-container-apps",
                    code = AcaControlErrorKind::DeadlineExpired.code(),
                    "credential lease acquisition timed out before provider call"
                );
                AcaControllerError::Effect(AcaControlErrorKind::DeadlineExpired)
            })?
            .map_err(|error| {
                tracing::warn!(
                    resource = %self.binding.guest_uid,
                    provider = "runtime-azure-container-apps",
                    code = error.kind().code(),
                    "credential lease acquisition failed"
                );
                AcaControllerError::Effect(error.kind())
            })?;
        if lease.expires_at_unix_ms() <= self.clock.now_unix_ms()
            || lease.expires_at_unix_ms() < request.requested_expiry_unix_ms()
        {
            if timeout_at(deadline, self.leases.revoke(&lease))
                .await
                .map(|revocation| revocation.is_err())
                .unwrap_or(true)
            {
                tracing::warn!(
                    resource = %self.binding.guest_uid,
                    provider = "runtime-azure-container-apps",
                    "stale credential lease revocation failed"
                );
            }
            tracing::warn!(
                resource = %self.binding.guest_uid,
                provider = "runtime-azure-container-apps",
                code = AcaControlErrorKind::DeadlineExpired.code(),
                "acquired credential lease already expired; operation rejected"
            );
            return Err(AcaControllerError::Effect(
                AcaControlErrorKind::DeadlineExpired,
            ));
        }
        let context = AcaControlContext::new(operation_id, deadline_remaining_ms);
        let result = timeout_at(
            deadline,
            call(Arc::clone(&self.control), lease.clone(), context),
        )
        .await
        .map_err(|_| {
            tracing::warn!(
                resource = %self.binding.guest_uid,
                provider = "runtime-azure-container-apps",
                code = AcaControlErrorKind::DeadlineExpired.code(),
                purpose = ?purpose,
                "provider operation deadline expired"
            );
            AcaControllerError::Effect(AcaControlErrorKind::DeadlineExpired)
        })
        .and_then(|result| {
            result.map_err(|error| {
                tracing::warn!(
                    resource = %self.binding.guest_uid,
                    provider = "runtime-azure-container-apps",
                    code = error.kind().code(),
                    purpose = ?purpose,
                    "provider operation failed"
                );
                AcaControllerError::Effect(error.kind())
            })
        });
        let revoke = timeout_at(deadline, self.leases.revoke(&lease))
            .await
            .map_err(|_| {
                tracing::warn!(
                    resource = %self.binding.guest_uid,
                    provider = "runtime-azure-container-apps",
                    "credential lease cleanup timed out"
                );
                AcaControllerError::LeaseCleanup(AcaControlErrorKind::DeadlineExpired)
            })
            .and_then(|result| {
                result.map_err(|error| {
                    tracing::warn!(
                        resource = %self.binding.guest_uid,
                        provider = "runtime-azure-container-apps",
                        code = error.kind().code(),
                        "credential lease cleanup failed after operation"
                    );
                    AcaControllerError::LeaseCleanup(error.kind())
                })
            });
        match (result, revoke) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn record(&mut self, operation_id: AcaOperationId) {
        self.ledger.record(
            operation_id,
            self.clock
                .now_unix_ms()
                .saturating_add(u64::from(self.config.plan_ttl_ms())),
            self.phase,
            self.config.completed_operation_capacity(),
        );
    }

    fn reset_readiness(&mut self) {
        self.readiness_generation = self.binding.provider_generation;
        self.readiness_attempts = 0;
        self.readiness_lifecycle = None;
    }

    fn readiness_retry(
        &mut self,
        lifecycle: AcaSandboxLifecycle,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        if self.readiness_generation != self.binding.provider_generation
            || self.readiness_lifecycle != Some(lifecycle)
        {
            self.readiness_generation = self.binding.provider_generation;
            self.readiness_attempts = 0;
            self.readiness_lifecycle = Some(lifecycle);
        }
        self.readiness_attempts = self.readiness_attempts.saturating_add(1);
        if self.readiness_attempts >= self.config.readiness().attempts() {
            tracing::warn!(
                resource = %self.binding.guest_uid,
                provider = "runtime-azure-container-apps",
                lifecycle = ?lifecycle,
                attempts = self.readiness_attempts,
                "readiness attempts exhausted; marking generation failed"
            );
            self.phase = AcaPhase::Failed;
            return Err(AcaControllerError::ReadinessExhausted);
        }
        tracing::debug!(
            resource = %self.binding.guest_uid,
            provider = "runtime-azure-container-apps",
            lifecycle = ?lifecycle,
            attempt = self.readiness_attempts,
            "readiness retry scheduled for non-ready sandbox"
        );
        self.phase = match lifecycle {
            AcaSandboxLifecycle::Creating | AcaSandboxLifecycle::Stopping => AcaPhase::Provisioning,
            AcaSandboxLifecycle::Failed | AcaSandboxLifecycle::Unknown => AcaPhase::Degraded,
            AcaSandboxLifecycle::Suspended | AcaSandboxLifecycle::Stopped => AcaPhase::Starting,
            AcaSandboxLifecycle::Running => AcaPhase::Degraded,
        };
        Ok(
            if matches!(
                lifecycle,
                AcaSandboxLifecycle::Creating | AcaSandboxLifecycle::Stopping
            ) {
                AcaReconcileOutcome::Progressing {
                    after_ms: self.config.readiness().interval_ms(),
                }
            } else {
                AcaReconcileOutcome::Retry {
                    after_ms: self.config.readiness().interval_ms(),
                }
            },
        )
    }

    const fn retryable_error(&self, error: AcaControllerError) -> bool {
        matches!(
            error,
            AcaControllerError::Effect(kind) if kind.retryable()
        )
    }

    fn finish_finalization(&mut self) {
        self.observed = None;
        self.finalizer = false;
        self.phase = AcaPhase::Finalized;
        self.finalization_stage = AcaFinalizationStage::Delete;
    }

    fn ensure_sandbox_generation(
        &self,
        record: &AcaSandboxRecord,
    ) -> Result<(), AcaControllerError> {
        if record.generation == self.binding.provider_generation {
            Ok(())
        } else {
            Err(AcaControllerError::InvalidState)
        }
    }

    fn completed_outcome(&self, operation_id: &AcaOperationId) -> Option<AcaReconcileOutcome> {
        match self.ledger.get(operation_id)? {
            AcaPhase::Ready => Some(AcaReconcileOutcome::Converged),
            _ => None,
        }
    }

    fn ensure_active(&self) -> Result<(), AcaControllerError> {
        if self.finalizer && !matches!(self.phase, AcaPhase::Finalizing | AcaPhase::Finalized) {
            Ok(())
        } else {
            Err(AcaControllerError::InvalidState)
        }
    }
}

fn one_candidate(
    candidates: AcaSandboxCandidates,
) -> Result<Option<AcaSandboxRecord>, AcaControllerError> {
    let mut candidates = candidates.into_iter();
    match (candidates.next(), candidates.next()) {
        (Some(candidate), None) => Ok(Some(candidate)),
        (None, None) => Ok(None),
        _ => Err(AcaControllerError::AmbiguousAdoption),
    }
}

fn one_disk_image(
    candidates: crate::AcaDiskImageCandidates,
    generation: u64,
) -> Result<Option<AcaDiskImageRecord>, AcaControlError> {
    let mut candidates = candidates.into_iter();
    match (candidates.next(), candidates.next()) {
        (Some(candidate), None) if candidate.generation == generation => Ok(Some(candidate)),
        (None, None) => Ok(None),
        (Some(_), None) => Err(AcaControlError::new(AcaControlErrorKind::Conflict)),
        _ => Err(AcaControlError::new(AcaControlErrorKind::Ambiguous)),
    }
}

/// Provider wrapper that binds the root config to injected effect ports.
pub struct AzureContainerAppsRuntimeProvider<C, L> {
    config: AcaProviderConfig,
    control: Arc<C>,
    leases: Arc<L>,
}

impl<C, L> AzureContainerAppsRuntimeProvider<C, L>
where
    C: AcaControl + 'static,
    L: AcaCredentialLeaseClient + 'static,
{
    /// Construct the provider. No SDK or ambient credential chain is opened.
    pub fn new(
        config: AcaProviderConfig,
        control: Arc<C>,
        leases: Arc<L>,
    ) -> Result<Self, AcaTypeError> {
        config.validate()?;
        Ok(Self {
            config,
            control,
            leases,
        })
    }

    /// Borrow the validated root configuration.
    pub const fn config(&self) -> &AcaProviderConfig {
        &self.config
    }

    /// Create a controller for one Guest using the provider defaults.
    pub fn controller(&self, binding: AcaResourceBinding) -> AcaController<C, L> {
        AcaController::new(
            binding,
            self.config.defaults.clone(),
            Arc::clone(&self.control),
            Arc::clone(&self.leases),
        )
        .with_provider_settings(
            self.config.network_ref.clone(),
            self.config.sandbox_transport_alias.clone(),
        )
    }
}
