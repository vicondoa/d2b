//! Azure VM lifecycle controller.

use std::sync::Arc;

use d2b_provider_toolkit::plane::{Clock, SystemClock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    bootstrap::{BootstrapPsk, BootstrapService, BootstrapServiceState},
    config::{AzureVmConfig, AzureVmGuestSettings},
    effect::AzureCredentialPort,
    effect::{
        AzureAccessToken, AzureEffectPort, AzureVmHandle, AzureVmState, LroStatus,
        PskExtensionPayload, TagDigest,
    },
    error::AzureVmError,
};

const MAX_PSK_DELIVERY_ATTEMPTS: u8 = 3;
const MAX_LRO_AGE_MS: u64 = 15 * 60 * 1_000;

/// Azure VM Provider phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AzureVmPhase {
    /// No correlated VM exists.
    Absent,
    /// VM provisioning is in progress.
    Provisioning,
    /// PSK extension delivery is in progress.
    PskDelivering,
    /// The one-time PSK extension is being removed.
    PskCleaning,
    /// VM is awaiting the bootstrap session.
    Bootstrapping,
    /// VM and enrolled KK session are ready.
    Ready,
    /// VM is draining.
    Draining,
    /// VM deletion is in progress.
    Deleting,
    /// Provider-owned child resources are being removed.
    ChildCleaning,
    /// Provider failed closed.
    Failed,
    /// Finalizer completed.
    Finalized,
}

/// Default descriptor repair interval.
pub const AZURE_VM_REPAIR_INTERVAL_SECS: u64 = 30;
/// Exact Guest finalizer owned by the Azure VM runtime Provider.
pub const AZURE_VM_GUEST_FINALIZER: &str =
    "runtime-azure-virtual-machine.d2bus.org/guest-cleanup";

/// Non-blocking controller result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmReconcileOutcome {
    /// The VM is ready.
    Converged,
    /// Poll again after a bounded delay.
    Progressing {
        /// Delay in milliseconds.
        after_ms: u32,
    },
    /// Retry the same operation.
    Retry {
        /// Delay in milliseconds.
        after_ms: u32,
    },
}

/// Opaque in-flight ARM operation together with its controller-local start
/// time. The two values are always Some-together/None-together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InFlightOperation {
    /// Opaque in-flight ARM operation.
    pub operation: crate::effect::AzureOperationHandle,
    /// Controller-local LRO start time.
    pub started_at: u64,
}

/// Non-secret controller state required for restart recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AzureVmRecoveryState {
    /// Current lifecycle phase.
    pub phase: AzureVmPhase,
    /// Whether the finalizer remains installed.
    pub finalizer_installed: bool,
    /// Opaque in-flight ARM operation and its start time.
    pub in_flight_operation: Option<InFlightOperation>,
    /// Deterministic delete operation id, when deletion is pending.
    pub pending_delete_operation_id: Option<String>,
    /// Bootstrap deadline start.
    pub bootstrap_started_at_unix_ms: Option<u64>,
    /// Number of extension delivery attempts.
    pub psk_delivery_attempts: u8,
    /// Bootstrap service enrollment state.
    pub bootstrap_service_state: BootstrapServiceState,
    /// Whether the one-time bootstrap extension may still contain PSK data.
    #[serde(default)]
    pub bootstrap_extension_present: bool,
    /// Whether provider-owned child-resource cleanup has completed.
    #[serde(default)]
    pub child_cleanup_complete: bool,
    /// Whether bootstrap expiry caused the current cleanup operation.
    #[serde(default)]
    pub bootstrap_deadline_failed: bool,
}

impl<'de> Deserialize<'de> for AzureVmRecoveryState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // The record is written by `recovery_state` and read back through
        // serde. The in-flight operation is one grouped object today, but
        // records written before the grouping carry the legacy
        // `operation` + `operationStartedAtUnixMs` pair; both shapes load
        // and the pair folds into the grouped shape when both members
        // are present. The write side always sets or clears both values
        // together, but a record read back with a half-Some pair is
        // malformed, and the decode refuses it below.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct NewShape {
            phase: AzureVmPhase,
            finalizer_installed: bool,
            in_flight_operation: Option<InFlightOperation>,
            pending_delete_operation_id: Option<String>,
            bootstrap_started_at_unix_ms: Option<u64>,
            psk_delivery_attempts: u8,
            bootstrap_service_state: BootstrapServiceState,
            #[serde(default)]
            bootstrap_extension_present: bool,
            #[serde(default)]
            child_cleanup_complete: bool,
            #[serde(default)]
            bootstrap_deadline_failed: bool,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct LegacyShape {
            phase: AzureVmPhase,
            finalizer_installed: bool,
            operation: Option<crate::effect::AzureOperationHandle>,
            pending_delete_operation_id: Option<String>,
            bootstrap_started_at_unix_ms: Option<u64>,
            psk_delivery_attempts: u8,
            operation_started_at_unix_ms: Option<u64>,
            // The pre-grouping record also carried the pending typed
            // update. That surface is gone from the record, so the
            // legacy member is read only to keep sealed records
            // loadable, and then dropped.
            #[serde(default)]
            pending_update: Option<serde::de::IgnoredAny>,
            bootstrap_service_state: BootstrapServiceState,
            #[serde(default)]
            bootstrap_extension_present: bool,
            #[serde(default)]
            child_cleanup_complete: bool,
            #[serde(default)]
            bootstrap_deadline_failed: bool,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            New(NewShape),
            Legacy(LegacyShape),
        }

        Ok(match Repr::deserialize(deserializer)? {
            Repr::New(shape) => Self {
                phase: shape.phase,
                finalizer_installed: shape.finalizer_installed,
                in_flight_operation: shape.in_flight_operation,
                pending_delete_operation_id: shape.pending_delete_operation_id,
                bootstrap_started_at_unix_ms: shape.bootstrap_started_at_unix_ms,
                psk_delivery_attempts: shape.psk_delivery_attempts,
                bootstrap_service_state: shape.bootstrap_service_state,
                bootstrap_extension_present: shape.bootstrap_extension_present,
                child_cleanup_complete: shape.child_cleanup_complete,
                bootstrap_deadline_failed: shape.bootstrap_deadline_failed,
            },
            Repr::Legacy(shape) => {
                // The legacy pending-update member has no reader left: the
                // controller's update surface is gone, and a record whose
                // pending update was set carries the removed reconfiguration
                // phase, which the phase decode refuses.
                let _ = shape.pending_update;
                // A legacy record carries the operation and its start stamp
                // as a pair. The write side emits both or neither, but this
                // is decoded data read back off disk, so a half-Some pair is
                // a malformed record: it fails the decode with a typed serde
                // error instead of aborting the thread.
                let in_flight_operation = match (shape.operation, shape.operation_started_at_unix_ms)
                {
                    (Some(operation), Some(started_at)) => {
                        Some(InFlightOperation { operation, started_at })
                    }
                    (None, None) => None,
                    (Some(_), None) | (None, Some(_)) => {
                        return Err(<D::Error as serde::de::Error>::custom(
                            "legacy recovery record has a half-Some operation pair",
                        ));
                    }
                };
                Self {
                    phase: shape.phase,
                    finalizer_installed: shape.finalizer_installed,
                    in_flight_operation,
                    pending_delete_operation_id: shape.pending_delete_operation_id,
                    bootstrap_started_at_unix_ms: shape.bootstrap_started_at_unix_ms,
                    psk_delivery_attempts: shape.psk_delivery_attempts,
                    bootstrap_service_state: shape.bootstrap_service_state,
                    bootstrap_extension_present: shape.bootstrap_extension_present,
                    child_cleanup_complete: shape.child_cleanup_complete,
                    bootstrap_deadline_failed: shape.bootstrap_deadline_failed,
                }
            }
        })
    }
}
/// Azure VM controller.
pub struct AzureVmController<E> {
    settings: AzureVmGuestSettings,
    effect: E,
    credentials: Arc<dyn AzureCredentialPort>,
    phase: AzureVmPhase,
    finalizer: bool,
    in_flight_operation: Option<InFlightOperation>,
    vm_handle: Option<AzureVmHandle>,
    expected_tag_digest: TagDigest,
    bootstrap_psk: Option<BootstrapPsk>,
    bootstrap_service: BootstrapService,
    pending_delete_operation_id: Option<String>,
    bootstrap_started_at_unix_ms: Option<u64>,
    psk_delivery_attempts: u8,
    clock: Arc<dyn Clock>,
    bootstrap_extension_present: bool,
    child_cleanup_complete: bool,
    bootstrap_deadline_failed: bool,
}

impl<E> AzureVmController<E>
where
    E: AzureEffectPort + 'static,
{
    /// Construct a controller after validating the two config layers.
    pub fn new(
        provider_config: AzureVmConfig,
        settings: AzureVmGuestSettings,
        effect: E,
        credentials: Arc<dyn AzureCredentialPort>,
        bootstrap_psk: Option<BootstrapPsk>,
    ) -> Result<Self, AzureVmError> {
        provider_config.validate()?;
        settings.validate()?;
        let expected_tag_digest = TagDigest::from_tags(&settings.azure_tags);
        Ok(Self {
            settings,
            effect,
            credentials,
            phase: AzureVmPhase::Absent,
            finalizer: true,
            in_flight_operation: None,
            vm_handle: None,
            expected_tag_digest,
            bootstrap_psk,
            bootstrap_service: BootstrapService::default(),
            pending_delete_operation_id: None,
            bootstrap_started_at_unix_ms: None,
            psk_delivery_attempts: 0,
            clock: Arc::new(SystemClock),
            bootstrap_extension_present: false,
            child_cleanup_complete: false,
            bootstrap_deadline_failed: false,
        })
    }

    /// Inject the durable bootstrap service state recovered by the gateway.
    pub fn with_bootstrap_service(mut self, bootstrap_service: BootstrapService) -> Self {
        self.bootstrap_service = bootstrap_service;
        self
    }

    /// Replace the wall clock used for bootstrap deadlines.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Export non-secret state for the core-owned sealed recovery record.
    pub fn recovery_state(&self) -> AzureVmRecoveryState {
        AzureVmRecoveryState {
            phase: self.phase,
            finalizer_installed: self.finalizer,
            in_flight_operation: self.in_flight_operation.clone(),
            pending_delete_operation_id: self.pending_delete_operation_id.clone(),
            bootstrap_started_at_unix_ms: self.bootstrap_started_at_unix_ms,
            psk_delivery_attempts: self.psk_delivery_attempts,
            bootstrap_service_state: self.bootstrap_service.state(),
            bootstrap_extension_present: self.bootstrap_extension_present,
            child_cleanup_complete: self.child_cleanup_complete,
            bootstrap_deadline_failed: self.bootstrap_deadline_failed,
        }
    }

    /// Restore non-secret state after the controller has been reconstructed.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmError::InvalidConfiguration`] when the recovery
    /// record is internally inconsistent (an in-flight operation under a
    /// phase that allows none, a finalizer that disagrees with the phase, or
    /// identifier bounds). The operation/start-stamp pairing is checked when
    /// the record is decoded, not here: a half-paired legacy record fails the
    /// decode instead of reaching this predicate.
    pub fn restore_recovery_state(
        mut self,
        recovery: AzureVmRecoveryState,
    ) -> Result<Self, AzureVmError> {
        if (matches!(
                recovery.phase,
                AzureVmPhase::PskCleaning | AzureVmPhase::ChildCleaning
            ) && recovery.in_flight_operation.is_none())
            || (!recovery.finalizer_installed && recovery.phase != AzureVmPhase::Finalized)
            || recovery.psk_delivery_attempts > MAX_PSK_DELIVERY_ATTEMPTS
            || recovery
                .pending_delete_operation_id
                .as_ref()
                .is_some_and(|id| {
                    id.is_empty()
                        || id.len() > 128
                        || !id.bytes().all(|byte| byte.is_ascii_graphic())
                })
        {
            return Err(AzureVmError::InvalidConfiguration);
        }
        self.phase = recovery.phase;
        self.finalizer = recovery.finalizer_installed;
        self.in_flight_operation = recovery.in_flight_operation;
        self.pending_delete_operation_id = recovery.pending_delete_operation_id;
        self.bootstrap_started_at_unix_ms = recovery.bootstrap_started_at_unix_ms;
        self.psk_delivery_attempts = recovery.psk_delivery_attempts;
        self.bootstrap_service = BootstrapService::from_state(recovery.bootstrap_service_state);
        self.bootstrap_extension_present = recovery.bootstrap_extension_present;
        self.child_cleanup_complete = recovery.child_cleanup_complete;
        self.bootstrap_deadline_failed = recovery.bootstrap_deadline_failed;
        Ok(self)
    }

    /// Return the current phase.
    pub const fn phase(&self) -> AzureVmPhase {
        self.phase
    }

    /// Return whether the finalizer remains installed.
    pub const fn finalizer_installed(&self) -> bool {
        self.finalizer
    }

    /// Reconcile without blocking on ARM polling.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmError::InvalidConfiguration`] when the finalizer is
    /// missing, [`AzureVmError::Ambiguous`] when the owned VM identity is
    /// absent or ambiguous, the transient ARM variants
    /// ([`AzureVmError::Transient`], throttling, quota, and network
    /// variants) for retryable effect failures, and the fatal variants
    /// (`BootstrapFailed`, `ArmProvisioningFailed`, `ArmCredentialDenied`)
    /// when an effect cannot be retried.
    #[tracing::instrument(skip_all, fields(provider = "runtime-azure-virtual-machine"))]
    pub async fn reconcile(
        &mut self,
        zone_uid: &str,
        guest_uid: &str,
        generation: u64,
    ) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if !self.finalizer {
            return Err(AzureVmError::InvalidConfiguration);
        }
        if let Some(operation) = self
            .in_flight_operation
            .as_ref()
            .map(|in_flight| in_flight.operation.clone())
        {
            return self.poll_operation(operation).await;
        }
        if self.bootstrap_deadline_failed && self.pending_delete_operation_id.is_none() {
            tracing::warn!(
                zone = %zone_uid,
                resource = %guest_uid,
                "bootstrap deadline previously failed; failing generation"
            );
            self.phase = AzureVmPhase::Failed;
            if self.bootstrap_extension_present {
                return self.start_extension_cleanup().await;
            }
            return Err(AzureVmError::BootstrapFailed);
        }
        if self.pending_delete_operation_id.is_some() {
            self.phase = AzureVmPhase::Deleting;
            return self.start_pending_delete().await;
        }
        let token = self.arm_token().await?;
        let (state, handle, tags) = self.effect.get_vm_state(&self.settings, &token).await?;
        match state {
            AzureVmState::Absent => {
                let operation_id =
                    operation_id(zone_uid, guest_uid, generation, "provision");
                let token = self.arm_token().await?;
                let operation = self
                    .effect
                    .start_vm_provision(&self.settings, &operation_id, &token)
                    .await?;
                self.set_operation(operation);
                self.phase = AzureVmPhase::Provisioning;
                Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
            }
            AzureVmState::Running => {
                if let Err(error) = self.verify_owned_vm(handle, tags, "reconcile") {
                    if error == AzureVmError::ArmResourceConflict {
                        self.phase = AzureVmPhase::Failed;
                    }
                    return Err(error);
                }
                if self.bootstrap_psk.is_some()
                    && self.bootstrap_service.state() != BootstrapServiceState::Enrolled
                {
                    self.start_psk_delivery().await
                } else {
                    self.ready_if_enrolled().await
                }
            }
            AzureVmState::Provisioning => {
                self.phase = AzureVmPhase::Provisioning;
                Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
            }
            AzureVmState::Stopped => {
                self.phase = AzureVmPhase::Draining;
                Ok(AzureVmReconcileOutcome::Retry { after_ms: 1_000 })
            }
            AzureVmState::Failed | AzureVmState::Unknown => {
                tracing::warn!(
                    zone = %zone_uid,
                    resource = %guest_uid,
                    state = ?state,
                    "VM provisioning state failed or unknown"
                );
                self.phase = AzureVmPhase::Failed;
                Err(AzureVmError::ArmProvisioningFailed)
            }
        }
    }

    /// Advance the current opaque long-running operation.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmError::InvalidOperationHandle`] when the supplied
    /// handle is not the current operation, and the ARM effect variants for
    /// retryable and fatal polling failures.
    #[tracing::instrument(skip_all, fields(provider = "runtime-azure-virtual-machine"))]
    pub async fn poll_operation(
        &mut self,
        operation: crate::effect::AzureOperationHandle,
    ) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if self.in_flight_operation.as_ref().map(|in_flight| &in_flight.operation)
            != Some(&operation)
        {
            tracing::warn!(
                "poll called with a foreign operation handle"
            );
            return Err(AzureVmError::InvalidOperationHandle);
        }
        if self.operation_expired() {
            tracing::warn!(
                phase = ?self.phase,
                "long-running operation exceeded maximum age; abandoning"
            );
            self.clear_operation();
            if self.pending_delete_operation_id.is_some() {
                self.phase = AzureVmPhase::Deleting;
                return self.start_pending_delete().await;
            }
            self.phase = AzureVmPhase::Failed;
            return Err(AzureVmError::ArmProvisioningFailed);
        }
        let token = self.arm_token().await?;
        match self.effect.poll_lro(&operation, &token).await? {
            LroStatus::InProgress { after_ms } => Ok(AzureVmReconcileOutcome::Progressing {
                after_ms: after_ms.max(1),
            }),
            LroStatus::Failed => {
                tracing::warn!(
                    phase = ?self.phase,
                    "long-running operation failed"
                );
                if self.phase == AzureVmPhase::PskCleaning {
                    self.clear_operation();
                    self.phase = AzureVmPhase::Failed;
                    return Err(AzureVmError::BootstrapFailed);
                }
                if self.phase == AzureVmPhase::ChildCleaning {
                    self.clear_operation();
                    self.phase = AzureVmPhase::Failed;
                    return Err(AzureVmError::Ambiguous);
                }
                self.clear_operation();
                if self.pending_delete_operation_id.is_some() {
                    self.phase = AzureVmPhase::Deleting;
                    return self.start_pending_delete().await;
                }
                if self.phase == AzureVmPhase::PskDelivering {
                    return self.start_psk_delivery().await;
                }
                self.phase = AzureVmPhase::Failed;
                Err(AzureVmError::ArmProvisioningFailed)
            }
            LroStatus::Succeeded => {
                self.clear_operation();
                match self.phase {
                    AzureVmPhase::Provisioning => {
                        if self.pending_delete_operation_id.is_some() {
                            self.phase = AzureVmPhase::Deleting;
                            return self.start_pending_delete().await;
                        }
                        let token = self.arm_token().await?;
                        let (state, handle, tags) =
                            self.effect.get_vm_state(&self.settings, &token).await?;
                        if state != AzureVmState::Running {
                            tracing::warn!(
                                state = ?state,
                                "VM not running after provision LRO succeeded"
                            );
                            self.phase = AzureVmPhase::Failed;
                            return Err(AzureVmError::ArmProvisioningFailed);
                        }
                        if let Err(error) = self.verify_owned_vm(handle, tags, "provision-complete")
                        {
                            self.phase = AzureVmPhase::Failed;
                            return Err(error);
                        }
                        if self.bootstrap_psk.is_some() {
                            self.start_psk_delivery().await
                        } else {
                            self.phase = AzureVmPhase::Bootstrapping;
                            Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
                        }
                    }
                    AzureVmPhase::PskDelivering => {
                        self.bootstrap_psk = None;
                        self.phase = AzureVmPhase::Bootstrapping;
                        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
                    }
                    AzureVmPhase::PskCleaning => {
                        self.bootstrap_extension_present = false;
                        self.bootstrap_psk = None;
                        if self.bootstrap_deadline_failed {
                            tracing::warn!(
                                "bootstrap deadline failed; refusing to mark VM ready"
                            );
                            self.phase = AzureVmPhase::Failed;
                            return Err(AzureVmError::BootstrapFailed);
                        }
                        if self.pending_delete_operation_id.is_some() {
                            self.phase = AzureVmPhase::Deleting;
                            return self.start_pending_delete().await;
                        }
                        self.phase = AzureVmPhase::Ready;
                        Ok(AzureVmReconcileOutcome::Converged)
                    }
                    AzureVmPhase::Deleting => self.start_pending_delete().await,
                    AzureVmPhase::ChildCleaning => {
                        self.child_cleanup_complete = true;
                        self.finalizer = false;
                        self.pending_delete_operation_id = None;
                        self.phase = AzureVmPhase::Finalized;
                        Ok(AzureVmReconcileOutcome::Converged)
                    }
                    _ => Ok(AzureVmReconcileOutcome::Converged),
                }
            }
        }
    }

    /// Begin deletion. The finalizer is retained until the LRO succeeds.
    ///
    /// # Errors
    ///
    /// Returns [`AzureVmError::Ambiguous`] when the owned VM identity or
    /// pending delete operation is absent, and the ARM effect variants for
    /// retryable and fatal deletion failures.
    #[tracing::instrument(skip_all, fields(provider = "runtime-azure-virtual-machine"))]
    pub async fn finalize(
        &mut self,
        zone_uid: &str,
        guest_uid: &str,
        generation: u64,
    ) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if !self.finalizer {
            return Ok(AzureVmReconcileOutcome::Converged);
        }
        let delete_operation_id = self
            .pending_delete_operation_id
            .get_or_insert_with(|| {
                operation_id(zone_uid, guest_uid, generation, "delete")
            })
            .clone();
        if self.in_flight_operation.is_some() {
            if !matches!(
                self.phase,
                AzureVmPhase::PskCleaning | AzureVmPhase::ChildCleaning
            ) {
                self.phase = AzureVmPhase::Deleting;
            }
            return Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 });
        }
        if self.bootstrap_extension_present {
            self.pending_delete_operation_id = Some(delete_operation_id);
            self.phase = AzureVmPhase::Deleting;
            return self.start_extension_cleanup().await;
        }
        let token = self.arm_token().await?;
        let (state, handle, tags) = self.effect.get_vm_state(&self.settings, &token).await?;
        let handle = match state {
            AzureVmState::Absent => return self.start_child_cleanup().await,
            AzureVmState::Running | AzureVmState::Stopped => {
                match self.verify_owned_vm(handle, tags, "finalization") {
                    Ok((handle, _)) => handle,
                    Err(error) => {
                        self.phase = AzureVmPhase::Failed;
                        return Err(error);
                    }
                }
            }
            AzureVmState::Provisioning => {
                self.phase = AzureVmPhase::Deleting;
                return Ok(AzureVmReconcileOutcome::Retry { after_ms: 1_000 });
            }
            AzureVmState::Failed | AzureVmState::Unknown => {
                tracing::warn!(
                    zone = %zone_uid,
                    resource = %guest_uid,
                    state = ?state,
                    "VM state failed or unknown during finalization"
                );
                self.phase = AzureVmPhase::Failed;
                return Err(AzureVmError::Transient);
            }
        };
        let token = self.arm_token().await?;
        let operation = self
            .effect
            .start_vm_delete(&handle, &delete_operation_id, &token)
            .await?;
        self.set_operation(operation);
        self.phase = AzureVmPhase::Deleting;
        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
    }

    async fn start_psk_delivery(&mut self) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        let handle = self.vm_handle.as_ref().ok_or(AzureVmError::Ambiguous)?;
        let started = *self
            .bootstrap_started_at_unix_ms
            .get_or_insert_with(|| self.clock.now_unix_ms());
        if self.clock.now_unix_ms().saturating_sub(started) >= self.settings.bootstrap_deadline_ms {
            tracing::warn!(
                "bootstrap PSK delivery deadline elapsed"
            );
            self.phase = AzureVmPhase::Failed;
            return Err(AzureVmError::BootstrapFailed);
        }
        if self.psk_delivery_attempts >= MAX_PSK_DELIVERY_ATTEMPTS {
            tracing::warn!(
                attempts = self.psk_delivery_attempts,
                "bootstrap PSK delivery attempts exhausted"
            );
            self.phase = AzureVmPhase::Failed;
            return Err(AzureVmError::BootstrapFailed);
        }
        let psk = self
            .bootstrap_psk
            .as_ref()
            .ok_or(AzureVmError::BootstrapFailed)?;
        let mut delivery = psk.copy_for_delivery();
        let payload = PskExtensionPayload::from_secret(std::mem::take(&mut *delivery))?;
        let token = self.arm_token().await?;
        let operation = self
            .effect
            .put_vm_extension(handle, payload, &token)
            .await?;
        self.psk_delivery_attempts = self.psk_delivery_attempts.saturating_add(1);
        self.bootstrap_extension_present = true;
        self.set_operation(operation);
        self.phase = AzureVmPhase::PskDelivering;
        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 250 })
    }

    async fn ready_if_enrolled(&mut self) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if self.bootstrap_service.state() != BootstrapServiceState::Enrolled {
            let started = *self
                .bootstrap_started_at_unix_ms
                .get_or_insert_with(|| self.clock.now_unix_ms());
            if self.clock.now_unix_ms().saturating_sub(started)
                >= self.settings.bootstrap_deadline_ms
            {
                tracing::warn!(
                    "bootstrap enrollment deadline elapsed before guest enrolled"
                );
                self.phase = AzureVmPhase::Failed;
                self.bootstrap_deadline_failed = true;
                if self.bootstrap_extension_present {
                    return self.start_extension_cleanup().await;
                }
                return Err(AzureVmError::BootstrapFailed);
            }
            self.phase = AzureVmPhase::Bootstrapping;
            return Ok(AzureVmReconcileOutcome::Retry { after_ms: 1_000 });
        }
        if self.bootstrap_extension_present {
            return self.start_extension_cleanup().await;
        }
        self.bootstrap_psk = None;
        self.phase = AzureVmPhase::Ready;
        Ok(AzureVmReconcileOutcome::Converged)
    }

    async fn start_pending_delete(&mut self) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if self.bootstrap_extension_present {
            return self.start_extension_cleanup().await;
        }
        let token = self.arm_token().await?;
        let (state, handle, tags) = self.effect.get_vm_state(&self.settings, &token).await?;
        match state {
            AzureVmState::Absent => self.start_child_cleanup().await,
            AzureVmState::Running | AzureVmState::Stopped => {
                let (handle, _) = self.verify_owned_vm(handle, tags, "pending-delete")?;
                let operation_id = self
                    .pending_delete_operation_id
                    .as_deref()
                    .ok_or(AzureVmError::Ambiguous)?;
                let token = self.arm_token().await?;
                let operation = self
                    .effect
                    .start_vm_delete(&handle, operation_id, &token)
                    .await?;
                self.set_operation(operation);
                self.phase = AzureVmPhase::Deleting;
                Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
            }

            AzureVmState::Provisioning => {
                self.phase = AzureVmPhase::Deleting;
                Ok(AzureVmReconcileOutcome::Retry { after_ms: 1_000 })
            }
            AzureVmState::Failed | AzureVmState::Unknown => {
                tracing::warn!(
                    state = ?state,
                    "VM state failed or unknown during pending delete"
                );
                self.phase = AzureVmPhase::Failed;
                Err(AzureVmError::Transient)
            }
        }
    }

    async fn start_extension_cleanup(&mut self) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if self.in_flight_operation.is_some() {
            return Ok(AzureVmReconcileOutcome::Progressing { after_ms: 250 });
        }
        let token = self.arm_token().await?;
        let operation = self
            .effect
            .delete_vm_extension(&self.settings, &token)
            .await?;
        self.set_operation(operation);
        self.phase = AzureVmPhase::PskCleaning;
        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 250 })
    }

    async fn start_child_cleanup(&mut self) -> Result<AzureVmReconcileOutcome, AzureVmError> {
        if self.child_cleanup_complete {
            self.finalizer = false;
            self.pending_delete_operation_id = None;
            self.phase = AzureVmPhase::Finalized;
            return Ok(AzureVmReconcileOutcome::Converged);
        }
        if self.in_flight_operation.is_some() {
            return Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 });
        }
        let operation_id = self
            .pending_delete_operation_id
            .as_deref()
            .ok_or(AzureVmError::Ambiguous)?;
        let token = self.arm_token().await?;
        let operation = self
            .effect
            .start_child_resource_cleanup(&self.settings, operation_id, &token)
            .await?;
        self.set_operation(operation);
        self.phase = AzureVmPhase::ChildCleaning;
        Ok(AzureVmReconcileOutcome::Progressing { after_ms: 1_000 })
    }

    /// Verify that an observed VM is the provider-owned one before acting on it.
    ///
    /// The tag digest is compared against the digest derived from the
    /// configured tags, so a VM carrying foreign or drifted ownership tags is
    /// never adopted, reconfigured, or deleted. Failures are logged here and
    /// returned as an error; `stage` names the caller's phase in the record.
    /// The observed handle is stored on success.
    fn verify_owned_vm(
        &mut self,
        handle: Option<AzureVmHandle>,
        tags: Option<TagDigest>,
        stage: &str,
    ) -> Result<(AzureVmHandle, TagDigest), AzureVmError> {
        let Some(handle) = handle else {
            tracing::warn!(
                stage,
                "running VM observed without effect handle"
            );
            return Err(AzureVmError::Ambiguous);
        };
        let Some(tags) = tags else {
            tracing::warn!(
                stage,
                "VM tag digest missing; refusing foreign or drifted resource"
            );
            return Err(AzureVmError::ArmResourceConflict);
        };
        if tags != self.expected_tag_digest {
            tracing::warn!(
                stage,
                "VM tag digest mismatch; refusing foreign or drifted resource"
            );
            return Err(AzureVmError::ArmResourceConflict);
        }
        self.vm_handle = Some(handle.clone());
        Ok((handle, tags))
    }

    fn set_operation(&mut self, operation: crate::effect::AzureOperationHandle) {
        self.in_flight_operation = Some(InFlightOperation {
            operation,
            started_at: self.clock.now_unix_ms(),
        });
    }

    fn clear_operation(&mut self) {
        self.in_flight_operation = None;
    }

    fn operation_expired(&self) -> bool {
        self.in_flight_operation.as_ref().is_some_and(|in_flight| {
            self.clock.now_unix_ms().saturating_sub(in_flight.started_at) >= MAX_LRO_AGE_MS
        })
    }

    async fn arm_token(&self) -> Result<AzureAccessToken, AzureVmError> {
        self.credentials
            .acquire_token("https://management.azure.com/", 30_000)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    code = error.code(),
                    "ARM access token acquisition failed"
                );
            })
    }
}

/// Derive a stable 20-character operation identifier.
fn operation_id(zone_uid: &str, guest_uid: &str, generation: u64, operation_class: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(zone_uid.as_bytes());
    digest.update([0]);
    digest.update(guest_uid.as_bytes());
    digest.update([0]);
    digest.update(generation.to_be_bytes());
    digest.update([0]);
    digest.update(operation_class.as_bytes());
    let mut id = base32(&digest.finalize());
    id.truncate(20);
    id
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut output = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0u16;
    let mut bits = 0u8;
    for byte in bytes {
        buffer = (buffer << 8) | u16::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits != 0 {
        output.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    output
}
