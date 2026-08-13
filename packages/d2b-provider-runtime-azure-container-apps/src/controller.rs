//! ACA Guest lifecycle controller.

use std::{collections::BTreeMap, fmt, sync::Arc};

use sha2::{Digest, Sha256};

use d2b_contracts::provider_effects::aca::{
    AcaControl, AcaControlContext, AcaControlError, AcaControlErrorKind, AcaCredentialLease,
    AcaCredentialLeaseClient, AcaCredentialLeaseRequest, AcaCredentialPurpose, AcaDesiredDiskImage,
    AcaDesiredSandbox, AcaDiskImageRecord, AcaOperationId, AcaProviderConfig, AcaResourceBinding,
    AcaRuntimeConfig, AcaSandboxCandidates, AcaSandboxLifecycle, AcaSandboxRecord,
    AcaWorkloadQuery,
};

/// Provider lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// No disk image was returned and creation was not possible.
    DiskImageUnavailable,
    /// No sandbox was available for a requested operation.
    SandboxUnavailable,
    /// The injected effect failed.
    Effect(AcaControlErrorKind),
    /// Credential cleanup failed after an otherwise successful operation.
    LeaseCleanup(AcaControlErrorKind),
}

impl AcaControllerError {
    /// Return the stable public error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidState => "aca-invalid-state",
            Self::AmbiguousAdoption => "aca-ambiguous-adoption",
            Self::DiskImageUnavailable => "aca-disk-image-unavailable",
            Self::SandboxUnavailable => "aca-sandbox-unavailable",
            Self::Effect(kind) | Self::LeaseCleanup(kind) => kind.code(),
        }
    }
}

impl fmt::Display for AcaControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AcaControllerError {}

/// Redacted status projected to the Guest resource.
#[derive(Clone, PartialEq, Eq)]
pub struct AcaStatus {
    phase: AcaPhase,
    identity_digest: Option<[u8; 32]>,
    observed_generation: u64,
}

impl AcaStatus {
    /// Return the lifecycle phase.
    pub const fn phase(&self) -> AcaPhase {
        self.phase
    }

    /// Return the bounded non-authorizing identity digest.
    pub const fn identity_digest(&self) -> Option<[u8; 32]> {
        self.identity_digest
    }

    /// Return the observed Provider generation.
    pub const fn observed_generation(&self) -> u64 {
        self.observed_generation
    }
}

impl fmt::Debug for AcaStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcaStatus")
            .field("phase", &self.phase)
            .field(
                "identity_digest",
                &self.identity_digest.map(|_| "<redacted>"),
            )
            .field("observed_generation", &self.observed_generation)
            .finish()
    }
}

/// Small bounded completed-operation ledger used by the controller adapter.
#[derive(Debug, Default)]
pub struct CompletedOperationLedger {
    completed: BTreeMap<AcaOperationId, (u64, AcaPhase)>,
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
        self.completed
            .insert(operation_id, (expires_at_unix_ms, phase));
        while self.completed.len() > capacity {
            let Some(oldest) = self.completed.keys().next().cloned() else {
                break;
            };
            self.completed.remove(&oldest);
        }
    }

    /// Remove expired operation records.
    pub fn prune(&mut self, now_unix_ms: u64) {
        self.completed
            .retain(|_, (expires_at, _)| *expires_at > now_unix_ms);
    }

    /// Return a previously completed phase.
    pub fn get(&self, operation_id: &AcaOperationId) -> Option<AcaPhase> {
        self.completed.get(operation_id).map(|(_, phase)| *phase)
    }

    /// Return the number of retained records.
    pub fn len(&self) -> usize {
        self.completed.len()
    }

    /// Return whether no records are retained.
    pub fn is_empty(&self) -> bool {
        self.completed.is_empty()
    }
}

/// Canonical ACA lifecycle controller.
pub struct AcaController<C, L> {
    binding: AcaResourceBinding,
    config: AcaRuntimeConfig,
    control: Arc<C>,
    leases: Arc<L>,
    phase: AcaPhase,
    finalizer: bool,
    observed: Option<AcaSandboxRecord>,
    disk_image: Option<AcaDiskImageRecord>,
    ledger: CompletedOperationLedger,
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
        Self {
            binding,
            config,
            control,
            leases,
            phase: AcaPhase::Pending,
            finalizer: true,
            observed: None,
            disk_image: None,
            ledger: CompletedOperationLedger::default(),
        }
    }

    /// Return the current phase.
    pub const fn phase(&self) -> AcaPhase {
        self.phase
    }

    /// Return whether the finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Return the current redacted status projection.
    pub fn status(&self) -> AcaStatus {
        AcaStatus {
            phase: self.phase,
            identity_digest: self.observed.as_ref().map(|record| {
                let mut digest = Sha256::new();
                digest.update(record.id.as_str().as_bytes());
                digest.update(self.binding.provider_generation.to_be_bytes());
                digest.update(self.binding.config_fingerprint);
                digest.finalize().into()
            }),
            observed_generation: self.binding.provider_generation,
        }
    }

    /// Return the bounded operation ledger.
    pub const fn ledger(&self) -> &CompletedOperationLedger {
        &self.ledger
    }

    /// Reconcile using external observation before any ensure effect.
    pub async fn reconcile(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        self.ensure_active()?;
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
                self.phase = AcaPhase::Degraded;
                return Err(error);
            }
        };
        match candidate {
            Some(record) => {
                self.reconcile_observed(operation_id, deadline_remaining_ms, record)
                    .await
            }
            None => {
                self.ensure_sandbox(operation_id, deadline_remaining_ms)
                    .await
            }
        }
    }

    /// Re-derive state without creating a missing sandbox.
    pub async fn adopt(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        self.ensure_active()?;
        let query = AcaWorkloadQuery {
            binding: self.binding.clone(),
            profile_id: self.config.profile().profile_id().clone(),
        };
        let candidates = self
            .with_lease(
                operation_id,
                AcaCredentialPurpose::Adopt,
                deadline_remaining_ms,
                move |control, lease, context| async move {
                    control.find_sandboxes(&lease, &context, &query).await
                },
            )
            .await?;
        let candidate = match one_candidate(candidates) {
            Ok(Some(candidate)) => candidate,
            Ok(None) => return Err(AcaControllerError::SandboxUnavailable),
            Err(error) => {
                self.phase = AcaPhase::Degraded;
                return Err(error);
            }
        };
        self.observed = Some(candidate.clone());
        if candidate.lifecycle == AcaSandboxLifecycle::Running {
            self.phase = AcaPhase::Ready;
            Ok(AcaReconcileOutcome::Converged)
        } else {
            self.phase = AcaPhase::Degraded;
            Ok(AcaReconcileOutcome::Retry {
                after_ms: self.config.readiness().interval_ms(),
            })
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
        if let Some(record) = self.observed.clone() {
            self.with_lease(
                operation_id.clone(),
                AcaCredentialPurpose::Destroy,
                deadline_remaining_ms,
                move |control, lease, context| async move {
                    if matches!(
                        record.lifecycle,
                        AcaSandboxLifecycle::Running | AcaSandboxLifecycle::Suspended
                    ) {
                        control.stop_sandbox(&lease, &context, &record.id).await?;
                    }
                    control.delete_sandbox(&lease, &context, &record.id).await
                },
            )
            .await?;
        }
        self.observed = None;
        self.finalizer = false;
        self.phase = AcaPhase::Finalized;
        Ok(())
    }

    async fn reconcile_observed(
        &mut self,
        operation_id: AcaOperationId,
        deadline_remaining_ms: u32,
        record: AcaSandboxRecord,
    ) -> Result<AcaReconcileOutcome, AcaControllerError> {
        self.observed = Some(record.clone());
        match record.lifecycle {
            AcaSandboxLifecycle::Running => {
                self.phase = AcaPhase::Ready;
                self.record(operation_id);
                Ok(AcaReconcileOutcome::Converged)
            }
            AcaSandboxLifecycle::Suspended | AcaSandboxLifecycle::Stopped => {
                self.phase = AcaPhase::Starting;
                let id = record.id.clone();
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
                self.phase = AcaPhase::Ready;
                self.record(operation_id);
                Ok(AcaReconcileOutcome::Converged)
            }
            AcaSandboxLifecycle::Creating | AcaSandboxLifecycle::Stopping => {
                self.phase = AcaPhase::Provisioning;
                Ok(AcaReconcileOutcome::Progressing {
                    after_ms: self.config.readiness().interval_ms(),
                })
            }
            AcaSandboxLifecycle::Failed | AcaSandboxLifecycle::Unknown => {
                self.phase = AcaPhase::Degraded;
                Ok(AcaReconcileOutcome::Retry {
                    after_ms: self.config.readiness().interval_ms(),
                })
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
        let image = self
            .with_lease(
                operation_id.clone(),
                AcaCredentialPurpose::Ensure,
                deadline_remaining_ms,
                move |control, lease, context| async move {
                    let candidates = control
                        .find_disk_images(&lease, &context, &desired_disk)
                        .await?;
                    if let Some(record) = candidates.as_slice().first().cloned() {
                        Ok(record)
                    } else {
                        control
                            .create_disk_image(&lease, &context, &desired_disk)
                            .await
                    }
                },
            )
            .await?;
        self.disk_image = Some(image.clone());
        let desired = AcaDesiredSandbox {
            binding: self.binding.clone(),
            profile: self.config.profile().clone(),
            disk_image: image,
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
        self.record(operation_id);
        Ok(AcaReconcileOutcome::Progressing {
            after_ms: self.config.readiness().interval_ms(),
        })
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
            return Err(AcaControllerError::Effect(
                AcaControlErrorKind::DeadlineExpired,
            ));
        }
        let request = AcaCredentialLeaseRequest::new(
            operation_id.clone(),
            purpose,
            u64::from(deadline_remaining_ms),
        );
        let lease = self
            .leases
            .acquire(&request)
            .await
            .map_err(|error| AcaControllerError::Effect(error.kind()))?;
        let context = AcaControlContext::new(operation_id, deadline_remaining_ms);
        let result = call(Arc::clone(&self.control), lease.clone(), context).await;
        let revoke = self.leases.revoke(&lease).await;
        match (result, revoke) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(AcaControllerError::Effect(error.kind())),
            (Ok(_), Err(error)) => Err(AcaControllerError::LeaseCleanup(error.kind())),
        }
    }

    fn record(&mut self, operation_id: AcaOperationId) {
        self.ledger.record(
            operation_id,
            u64::from(self.config.plan_ttl_ms()),
            self.phase,
            self.config.completed_operation_capacity(),
        );
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
    match candidates.as_slice() {
        [] => Ok(None),
        [candidate] => Ok(Some(candidate.clone())),
        _ => Err(AcaControllerError::AmbiguousAdoption),
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
    pub fn new(config: AcaProviderConfig, control: Arc<C>, leases: Arc<L>) -> Self {
        Self {
            config,
            control,
            leases,
        }
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
    }
}
