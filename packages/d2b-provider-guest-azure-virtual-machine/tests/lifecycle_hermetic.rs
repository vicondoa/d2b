use std::sync::Arc;
use tokio::sync::Mutex;

use async_trait::async_trait;
use d2b_contracts::{OpaqueAzureRef, ResourceRef};
use d2b_provider_guest_azure_virtual_machine::{
    AzureAccessToken, AzureCredentialPort, AzureEffectPort, AzureOperationHandle, AzureVmConfig,
    AzureVmController, AzureVmError, AzureVmGuestSettings, AzureVmHandle, AzureVmPhase,
    AzureVmReconcileOutcome, AzureVmRecoveryState, AzureVmState, BootstrapAdmission,
    BootstrapPsk, BootstrapPskDelivery, BootstrapService, DiskSku, LroStatus, PskExtensionPayload,
    TagDigest,
};
use d2b_provider_toolkit::plane::Clock;

struct FakeState {
    state: AzureVmState,
    handle: Option<AzureVmHandle>,
    tags: Option<TagDigest>,
    calls: Vec<&'static str>,
    polls: Vec<LroStatus>,
    extension_failures: usize,
    extension_delete_failures: usize,
    provision_failures: usize,
    provision_operation_ids: Vec<String>,
    stored_provisions: usize,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            state: AzureVmState::Absent,
            handle: None,
            tags: None,
            calls: Vec::new(),
            polls: Vec::new(),
            extension_failures: 0,
            extension_delete_failures: 0,
            provision_failures: 0,
            provision_operation_ids: Vec::new(),
            stored_provisions: 0,
        }
    }
}

struct FakeEffect {
    state: Arc<Mutex<FakeState>>,
}

struct FakeCredential;

#[async_trait]
impl AzureCredentialPort for FakeCredential {
    async fn acquire_token(
        &self,
        audience: &str,
        deadline_ms: u32,
    ) -> Result<AzureAccessToken, AzureVmError> {
        assert_eq!(audience, "https://management.azure.com/");
        assert!(deadline_ms > 0);
        Ok(zeroize::Zeroizing::new(b"arm-token".to_vec()))
    }
}

struct FixedClock(Arc<Mutex<u64>>);

impl Clock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        *self.0.try_lock().unwrap()
    }
}

#[async_trait]
impl AzureEffectPort for FakeEffect {
    async fn start_vm_provision(
        &self,
        _: &AzureVmGuestSettings,
        operation_id: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().await;
        state.calls.push("provision");
        // ARM is keyed on the idempotency token: a repeated token addresses
        // the machine the first call already made.
        state.provision_operation_ids.push(operation_id.to_owned());
        if state.provision_operation_ids.len() > 1 {
            let _ = state.provision_operation_ids.dedup_by(|_, _| false);
        }
        if state.stored_provisions > 0 {
            return Err(AzureVmError::ArmResourceConflict);
        }
        state.stored_provisions += 1;
        state.state = AzureVmState::Running;
        state.handle = Some(AzureVmHandle::from_core("opaque-vm").unwrap());
        state.tags = Some(TagDigest::from_tags(&[(
            "owner".to_owned(),
            "d2b".to_owned(),
        )]));
        if state.provision_failures > 0 {
            // The machine really exists; only the response is lost.
            state.provision_failures -= 1;
            return Err(AzureVmError::Transient);
        }
        Ok(AzureOperationHandle::from_core(b"provision").unwrap())
    }

    async fn poll_lro(
        &self,
        _: &AzureOperationHandle,
        _: &AzureAccessToken,
    ) -> Result<LroStatus, AzureVmError> {
        self.state
            .lock()
            .await
            .polls
            .pop()
            .ok_or(AzureVmError::Transient)
    }

    async fn get_vm_state(
        &self,
        _: &AzureVmGuestSettings,
        _: &AzureAccessToken,
    ) -> Result<(AzureVmState, Option<AzureVmHandle>, Option<TagDigest>), AzureVmError> {
        let state = self.state.lock().await;
        Ok((state.state, state.handle.clone(), state.tags))
    }

    async fn put_vm_extension(
        &self,
        _: &AzureVmHandle,
        _: PskExtensionPayload,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().await;
        state.calls.push("extension");
        if state.extension_failures > 0 {
            state.extension_failures -= 1;
            return Err(AzureVmError::Transient);
        }
        Ok(AzureOperationHandle::from_core(b"extension").unwrap())
    }

    async fn delete_vm_extension(
        &self,
        _: &AzureVmGuestSettings,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().await;
        state.calls.push("extension-delete");
        if state.extension_delete_failures > 0 {
            state.extension_delete_failures -= 1;
            return Err(AzureVmError::Transient);
        }
        Ok(AzureOperationHandle::from_core(b"extension-delete").unwrap())
    }

    async fn start_vm_resize(
        &self,
        _: &AzureVmHandle,
        _: &str,
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"resize").unwrap())
    }

    async fn start_vm_delete(
        &self,
        _: &AzureVmHandle,
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().await;
        state.calls.push("delete");
        state.state = AzureVmState::Absent;
        state.handle = None;
        state.tags = None;
        Ok(AzureOperationHandle::from_core(b"delete").unwrap())
    }

    async fn start_child_resource_cleanup(
        &self,
        _: &AzureVmGuestSettings,
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        self.state.lock().await.calls.push("child-cleanup");
        Ok(AzureOperationHandle::from_core(b"child-cleanup").unwrap())
    }

    async fn start_disk_attach(
        &self,
        _: &AzureVmHandle,
        _: &d2b_provider_guest_azure_virtual_machine::DataDiskSpec,
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"attach").unwrap())
    }

    async fn start_disk_detach(
        &self,
        _: &AzureVmHandle,
        _: u8,
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"detach").unwrap())
    }

    async fn update_vm_tags(
        &self,
        _: &AzureVmHandle,
        _: &[(String, String)],
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"tags").unwrap())
    }
}

fn config() -> (AzureVmConfig, AzureVmGuestSettings) {
    (
        AzureVmConfig {
            tenant_id: Some(OpaqueAzureRef::parse("tenant").unwrap()),
            client_id: None,
            arm_credential_ref: ResourceRef::parse("Credential/arm").unwrap(),
            controller_execution_ref: ResourceRef::parse("Guest/gateway").unwrap(),
            network_ref: Some(ResourceRef::parse("Network/egress").unwrap()),
        },
        AzureVmGuestSettings {
            subscription_id: OpaqueAzureRef::parse("subscription").unwrap(),
            resource_group: OpaqueAzureRef::parse("resource-group").unwrap(),
            region: OpaqueAzureRef::parse("eastus").unwrap(),
            vm_size: OpaqueAzureRef::parse("standard-d4").unwrap(),
            image_ref: OpaqueAzureRef::parse("image-1").unwrap(),
            disk_sku: DiskSku::PremiumLrs,
            os_disk_size_gb: Some(64),
            admin_user: "azureuser".to_owned(),
            vnet_subscription_id: None,
            vnet_resource_group: None,
            vnet_name: OpaqueAzureRef::parse("vnet").unwrap(),
            subnet_name: OpaqueAzureRef::parse("guests").unwrap(),
            assign_public_ip: false,
            data_disks: Vec::new(),
            bootstrap_psk_delivery: BootstrapPskDelivery::VmExtension,
            bootstrap_deadline_ms: 60_000,
            child_zone_hosting: false,
            azure_tags: vec![("owner".to_owned(), "d2b".to_owned())],
        },
    )
}

fn enrolled_service() -> BootstrapService {
    let mut service = BootstrapService::default();
    let mut admission =
        BootstrapAdmission::new(BootstrapPsk::from_bytes(b"enrollment").unwrap(), 10);
    service
        .complete_enrollment(&mut admission, b"enrollment", 1)
        .unwrap();
    service
}

fn expected_tag_digest() -> TagDigest {
    TagDigest::from_tags(&[("owner".to_owned(), "d2b".to_owned())])
}

fn credential() -> Arc<dyn AzureCredentialPort> {
    Arc::new(FakeCredential)
}

#[test]
fn azure_wire_enums_use_adr_values() {
    assert_eq!(
        serde_json::to_string(&DiskSku::PremiumLrs).unwrap(),
        "\"Premium_LRS\""
    );
    assert_eq!(
        serde_json::to_string(&BootstrapPskDelivery::VmExtension).unwrap(),
        "\"vm-extension\""
    );
    assert!(serde_json::from_str::<BootstrapPskDelivery>("\"user-data\"").is_err());
    assert_eq!(
        serde_json::from_str::<DiskSku>("\"StandardSSD_LRS\"").unwrap(),
        DiskSku::StandardSsdLrs
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn absent_vm_starts_non_blocking_provision() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(
        provider,
        settings,
        effect,
        credential(),
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap();
    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::Provisioning);
    assert_eq!(state.lock().await.calls, ["provision"]);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn observed_provisioning_vm_is_not_provisioned_again_after_restart() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Provisioning,
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller =
        AzureVmController::new(provider, settings, effect, credential(), None).unwrap();

    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::Provisioning);
    assert!(state.lock().await.calls.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn poll_rejects_an_operation_handle_that_is_not_current() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState::default()));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller =
        AzureVmController::new(provider, settings, effect, credential(), None).unwrap();
    controller.reconcile("zone", "guest", 1).await.unwrap();

    assert_eq!(
        controller
            .poll_operation(AzureOperationHandle::from_core(b"foreign").unwrap())
            .await
            .unwrap_err(),
        AzureVmError::InvalidOperationHandle
    );
    assert_eq!(controller.phase(), AzureVmPhase::Provisioning);
    assert_eq!(state.lock().await.calls, ["provision"]);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn finalize_preserves_the_first_delete_operation_id() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    let effect = FakeEffect { state };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());

    controller.finalize("zone", "guest", 1).await.unwrap();
    let first = controller.recovery_state().pending_delete_operation_id;
    controller.finalize("zone", "guest", 2).await.unwrap();
    assert_eq!(
        controller.recovery_state().pending_delete_operation_id,
        first
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn recovery_state_restores_opaque_lro_without_secret_material() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        polls: vec![LroStatus::Succeeded, LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let controller = AzureVmController::new(
        provider.clone(),
        settings.clone(),
        FakeEffect {
            state: Arc::clone(&state),
        },
        credential(),
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap();
    let mut controller = controller;
    controller.reconcile("zone", "guest", 1).await.unwrap();
    let recovery = controller.recovery_state();
    let encoded = serde_json::to_string(&recovery).unwrap();
    assert!(!encoded.contains("one-time"));
    assert!(encoded.contains("cHJvdmlzaW9u"));

    let mut restored = AzureVmController::new(
        provider,
        settings,
        FakeEffect {
            state: Arc::clone(&state),
        },
        credential(),
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap()
    .restore_recovery_state(recovery)
    .unwrap();
    assert_eq!(restored.phase(), AzureVmPhase::Provisioning);
    restored.reconcile("zone", "guest", 1).await.unwrap();
}

#[test]
fn legacy_operation_pair_recovery_record_still_loads() {
    // Records written before the in-flight operation was grouped carry the
    // `operation` + `operationStartedAtUnixMs` pair instead of
    // `inFlightOperation`; they must still load, folding into the grouped
    // shape, and re-serialize in the grouped shape.
    let recovery: AzureVmRecoveryState = serde_json::from_value(serde_json::json!({
        "phase": "provisioning",
        "finalizerInstalled": true,
        "operation": "cHJvdmlzaW9u",
        "pendingDeleteOperationId": null,
        "bootstrapStartedAtUnixMs": null,
        "pskDeliveryAttempts": 0,
        "operationStartedAtUnixMs": 42,
        "pendingUpdate": null,
        "bootstrapServiceState": "Waiting",
        "bootstrapExtensionPresent": false,
        "childCleanupComplete": false,
        "bootstrapDeadlineFailed": false,
    }))
    .unwrap();
    let in_flight = recovery
        .in_flight_operation
        .clone()
        .expect("legacy pair folds into the grouped shape");
    assert_eq!(
        in_flight.operation,
        AzureOperationHandle::from_core(b"provision").unwrap()
    );
    assert_eq!(in_flight.started_at, 42);
    assert_eq!(recovery.phase, AzureVmPhase::Provisioning);

    let encoded = serde_json::to_string(&recovery).unwrap();
    assert!(encoded.contains("\"inFlightOperation\""));
    assert!(!encoded.contains("operationStartedAtUnixMs"));
    let reloaded: AzureVmRecoveryState = serde_json::from_str(&encoded).unwrap();
    assert_eq!(reloaded, recovery);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn restart_converges_only_tagged_running_vm() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    assert_eq!(
        controller.reconcile("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AzureVmPhase::Ready);
    assert_eq!(
        state.lock().await.calls,
        Vec::<&str>::new(),
        "a running tagged VM is converged in place, never provisioned again"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn delete_keeps_finalizer_until_lro_completion() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        polls: vec![LroStatus::Succeeded, LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = FakeEffect { state };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    controller.reconcile("zone", "guest", 1).await.unwrap();
    assert!(matches!(
        controller.finalize("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert!(controller.finalizer_installed());
    controller
        .poll_operation(AzureOperationHandle::from_core(b"delete").unwrap())
        .await
        .unwrap();
    assert!(controller.finalizer_installed());
    controller
        .poll_operation(AzureOperationHandle::from_core(b"child-cleanup").unwrap())
        .await
        .unwrap();
    assert!(!controller.finalizer_installed());
    assert_eq!(controller.phase(), AzureVmPhase::Finalized);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn running_vm_waits_for_authenticated_enrollment() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    let effect = FakeEffect { state };
    let mut controller =
        AzureVmController::new(provider, settings, effect, credential(), None).unwrap();
    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Retry { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::Bootstrapping);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn failed_lro_honors_pending_delete_intent() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        polls: vec![
            LroStatus::Succeeded,
            LroStatus::Succeeded,
            LroStatus::Failed,
        ],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    controller.reconcile("zone", "guest", 1).await.unwrap();
    controller.finalize("zone", "guest", 2).await.unwrap();
    assert_eq!(
        controller
            .poll_operation(AzureOperationHandle::from_core(b"provision").unwrap())
            .await
            .unwrap(),
        AzureVmReconcileOutcome::Progressing { after_ms: 1_000 }
    );
    assert_eq!(controller.phase(), AzureVmPhase::Deleting);
    assert!(controller.finalizer_installed());
    assert_eq!(state.lock().await.calls, ["provision", "delete"]);
    controller
        .poll_operation(AzureOperationHandle::from_core(b"delete").unwrap())
        .await
        .unwrap();
    controller
        .poll_operation(AzureOperationHandle::from_core(b"child-cleanup").unwrap())
        .await
        .unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::Finalized);
    assert!(!controller.finalizer_installed());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn restart_with_pending_delete_never_reprovisions_an_absent_vm() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        polls: vec![LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .restore_recovery_state(AzureVmRecoveryState {
            phase: AzureVmPhase::Deleting,
            finalizer_installed: true,
            in_flight_operation: None,
            pending_delete_operation_id: Some("delete-id".to_owned()),
            bootstrap_started_at_unix_ms: None,
            psk_delivery_attempts: 0,
            bootstrap_service_state: BootstrapService::default().state(),
            bootstrap_extension_present: false,
            admitted_identity: None,
        child_cleanup_complete: false,
            bootstrap_deadline_failed: false,
        })
        .unwrap();
    let mut controller = controller;

    assert!(matches!(
        controller.reconcile("zone", "guest", 2).await.unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::ChildCleaning);
    controller
        .poll_operation(AzureOperationHandle::from_core(b"child-cleanup").unwrap())
        .await
        .unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::Finalized);
    assert_eq!(state.lock().await.calls, ["child-cleanup"]);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn foreign_tags_are_not_reconciled() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(TagDigest::from_core([9; 32])),
        ..FakeState::default()
    }));
    let effect = FakeEffect { state };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    assert_eq!(
        controller.reconcile("zone", "guest", 1).await.unwrap_err(),
        AzureVmError::ArmResourceConflict
    );
    assert_eq!(controller.phase(), AzureVmPhase::Failed);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn restart_finalization_reobserves_before_clearing_finalizer() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        polls: vec![LroStatus::Succeeded, LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    assert!(matches!(
        controller.finalize("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert!(controller.finalizer_installed());
    controller
        .poll_operation(AzureOperationHandle::from_core(b"delete").unwrap())
        .await
        .unwrap();
    controller
        .poll_operation(AzureOperationHandle::from_core(b"child-cleanup").unwrap())
        .await
        .unwrap();
    assert!(!controller.finalizer_installed());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn provisioning_lro_delivers_psk_before_bootstrap_phase() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        polls: vec![LroStatus::Succeeded, LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(
        provider,
        settings,
        effect,
        credential(),
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap();
    controller.reconcile("zone", "guest", 1).await.unwrap();
    controller
        .poll_operation(AzureOperationHandle::from_core(b"provision").unwrap())
        .await
        .unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::PskDelivering);
    controller
        .poll_operation(AzureOperationHandle::from_core(b"extension").unwrap())
        .await
        .unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::Bootstrapping);
    assert_eq!(state.lock().await.calls, ["provision", "extension"]);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn failed_extension_lro_redelivers_psk_without_losing_secret() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        polls: vec![
            LroStatus::Succeeded,
            LroStatus::Failed,
            LroStatus::Succeeded,
        ],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(
        provider,
        settings,
        effect,
        credential(),
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap();
    controller.reconcile("zone", "guest", 1).await.unwrap();
    controller
        .poll_operation(AzureOperationHandle::from_core(b"provision").unwrap())
        .await
        .unwrap();
    controller
        .poll_operation(AzureOperationHandle::from_core(b"extension").unwrap())
        .await
        .unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::PskDelivering);
    controller
        .poll_operation(AzureOperationHandle::from_core(b"extension").unwrap())
        .await
        .unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::Bootstrapping);
    assert_eq!(
        state.lock().await.calls,
        ["provision", "extension", "extension"]
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn transient_extension_failure_does_not_consume_delivery_attempt() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        extension_failures: 1,
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(
        provider,
        settings,
        effect,
        credential(),
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap();

    assert_eq!(
        controller.reconcile("zone", "guest", 1).await,
        Err(AzureVmError::Transient)
    );
    let recovery = controller.recovery_state();
    assert_eq!(recovery.psk_delivery_attempts, 0);
    assert!(!recovery.bootstrap_extension_present);

    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await,
        Ok(AzureVmReconcileOutcome::Progressing { .. })
    ));
    let recovery = controller.recovery_state();
    assert_eq!(recovery.psk_delivery_attempts, 1);
    assert!(recovery.bootstrap_extension_present);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn running_vm_fails_closed_at_bootstrap_deadline() {
    let (provider, settings) = config();
    let now = Arc::new(Mutex::new(0));
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    let effect = FakeEffect { state };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_clock(Arc::new(FixedClock(Arc::clone(&now))));
    controller.reconcile("zone", "guest", 1).await.unwrap();
    *now.lock().await = 60_000;
    assert_eq!(
        controller.reconcile("zone", "guest", 2).await.unwrap_err(),
        AzureVmError::BootstrapFailed
    );
    assert_eq!(controller.phase(), AzureVmPhase::Failed);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn bootstrap_deadline_retries_failed_extension_cleanup() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        extension_delete_failures: 1,
        polls: vec![LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let now = Arc::new(Mutex::new(60_000));
    let controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_clock(Arc::new(FixedClock(now)));
    let recovery = AzureVmRecoveryState {
        phase: AzureVmPhase::Failed,
        finalizer_installed: true,
        in_flight_operation: None,
        pending_delete_operation_id: None,
        bootstrap_started_at_unix_ms: Some(0),
        psk_delivery_attempts: 0,
        bootstrap_service_state: BootstrapService::default().state(),
        bootstrap_extension_present: true,
        admitted_identity: None,
        child_cleanup_complete: false,
        bootstrap_deadline_failed: true,
    };
    let mut controller = controller.restore_recovery_state(recovery).unwrap();

    assert_eq!(
        controller.reconcile("zone", "guest", 1).await,
        Err(AzureVmError::Transient)
    );
    assert!(controller.recovery_state().bootstrap_extension_present);

    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await,
        Ok(AzureVmReconcileOutcome::Progressing { .. })
    ));
    assert_eq!(
        controller
            .poll_operation(AzureOperationHandle::from_core(b"extension-delete").unwrap())
            .await,
        Err(AzureVmError::BootstrapFailed)
    );
    assert!(!controller.recovery_state().bootstrap_extension_present);
}

// ---------------------------------------------------------------------------
// The admitted remote authority
//
// Everything below drives the controller through the converted entry point:
// the graph admitted one Guest, one subscription, and one credential
// relationship, and every ARM call has to survive all three before the
// control plane is contacted.
// ---------------------------------------------------------------------------

use d2b_contracts_provider::v3::credential::{DeliveryRouteDigest, MAX_DELIVERY_RECORD_BYTES};
use d2b_contracts_provider::v3::{
    AdmittedCredentialDelivery, AudienceToken, CredentialAuthorization,
    CredentialDeliveryEvidence, CredentialMethod, DeliveryIdentity, DeliverySessionParams,
    OperationClass, PresentationCapability,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingRefusal, RefusalReason, ResourceGeneration, ResourceUid, ZoneId,
};
use d2b_provider_guest_azure_virtual_machine::{
    AZURE_VM_ARTIFACT_ID, AZURE_VM_CONTROL_AUDIENCE, AdmittedRecoveryIdentity,
    AzureVmAdmittedGuest, AzureVmAdmittedRemote, AzureVmCloudIdentity, AzureVmDeliveryContext,
    AzureVmRemoteAuthority, AzureVmRemoteDeliveryPort, AzureVmRemotePurpose, AzureVmRemoteRefusal,
    azure_vm_declaration, declared_presentation,
};

const ADMITTED_ZONE: &str = "work";
const ADMITTED_GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const OTHER_GUEST_UID: &str = "223e4567-e89b-42d3-a456-426614174000";
const ROUTE_DIGEST: &str =
    "sha256:6f1c1b6f2a6f2cbb1f2e5b2f0c3a7a2b6c9d0e1f2a3b4c5d6e7f809a1b2c3d4e";

fn graph_reference(value: &str) -> d2b_contracts_resource::v3::ResourceRef {
    d2b_contracts_resource::v3::ResourceRef::parse(value).expect("canonical reference")
}

fn graph_uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical uid")
}

fn generation(value: u64) -> ResourceGeneration {
    ResourceGeneration::new(value).expect("nonzero generation")
}

/// The `CredentialBinding` relationship the graph admitted, held exactly as
/// the source side committed it.
struct AdmittedRelationship {
    audience: AudienceToken,
    granted: Vec<OperationClass>,
    current: DeliveryIdentity,
    refusal: Option<BindingRefusal>,
}

impl AdmittedRelationship {
    fn live() -> Self {
        Self {
            audience: AudienceToken::parse(AZURE_VM_CONTROL_AUDIENCE).expect("ARM audience"),
            granted: vec![OperationClass::AcquireToken],
            current: delivery_identity(1, AZURE_VM_CONTROL_AUDIENCE),
            refusal: None,
        }
    }

    /// The same relationship after the source rotated its Credential: the
    /// consumer generation the session carries has moved on.
    fn revoked() -> Self {
        Self {
            refusal: Some(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::StaleAuthority,
            )),
            ..Self::live()
        }
    }
}

impl AdmittedCredentialDelivery for AdmittedRelationship {
    fn audience(&self) -> &AudienceToken {
        &self.audience
    }

    fn grants(&self, class: OperationClass) -> bool {
        self.granted.contains(&class)
    }

    fn current_delivery(&self) -> DeliveryIdentity {
        self.current.clone()
    }

    fn fence_refusal(&self, _: &CredentialDeliveryEvidence) -> Option<BindingRefusal> {
        self.refusal
    }
}

fn delivery_identity(sequence: u64, audience: &str) -> DeliveryIdentity {
    DeliveryIdentity::new(
        graph_reference("Credential/arm"),
        graph_uid(ADMITTED_GUEST_UID),
        generation(1),
        graph_reference(d2b_provider_guest_azure_virtual_machine::PROVIDER_REF),
        generation(1),
        AudienceToken::parse(audience).expect("audience"),
        OperationClass::AcquireToken,
        sequence,
    )
}

fn delivery(sequence: u64, audience: &str) -> DeliverySessionParams {
    DeliverySessionParams::new(
        graph_reference("Credential/arm"),
        graph_uid(ADMITTED_GUEST_UID),
        generation(1),
        graph_reference(d2b_provider_guest_azure_virtual_machine::PROVIDER_REF),
        generation(1),
        AudienceToken::parse(audience).expect("audience"),
        OperationClass::AcquireToken,
        u64::MAX,
        u64::MAX,
        DeliveryRouteDigest::parse(ROUTE_DIGEST).expect("route digest"),
        MAX_DELIVERY_RECORD_BYTES as u32,
        sequence,
    )
    .expect("delivery session")
}

fn authorization(sequence: u64, audience: &str) -> CredentialAuthorization {
    CredentialAuthorization::new(
        CredentialMethod::AcquireToken,
        Some(delivery(sequence, audience)),
    )
    .expect("authorization")
}

fn evidence() -> CredentialDeliveryEvidence {
    CredentialDeliveryEvidence::new(generation(1), generation(1), generation(1), 1)
}

/// The credential adapter the daemon composition root fills with the real
/// authenticated client.
struct FakeDelivery {
    sequence: u64,
    audience: String,
    asked: Arc<Mutex<usize>>,
}

#[async_trait]
impl AzureVmRemoteDeliveryPort for FakeDelivery {
    async fn authorize(
        &self,
        purpose: AzureVmRemotePurpose,
        _: &ResourceUid,
        _: u64,
    ) -> Result<AzureVmDeliveryContext, AzureVmRemoteRefusal> {
        *self.asked.lock().await += 1;
        let method = purpose.credential_method();
        let authorization = if method.requires_delivery() {
            authorization(self.sequence, &self.audience)
        } else {
            // A read carries no material: the adapter authorizes the metadata
            // method with no delivery session at all.
            CredentialAuthorization::new(method, None).expect("metadata authorization")
        };
        Ok(AzureVmDeliveryContext::new(authorization, evidence()))
    }
}

fn admitted_guest(uid: &str, generation: u64) -> AzureVmAdmittedGuest {
    AzureVmAdmittedGuest::new(
        ZoneId::parse(ADMITTED_ZONE).expect("zone"),
        graph_reference("Guest/workload"),
        graph_uid(uid),
        graph_reference(d2b_provider_guest_azure_virtual_machine::PROVIDER_REF),
        generation,
    )
    .expect("the evidence names this provider")
}

fn azure_cloud(subscription: &str, resource_group: &str) -> AzureVmCloudIdentity {
    AzureVmCloudIdentity::from_parts(
        Some("tenant".to_owned()),
        None,
        subscription.to_owned(),
        resource_group.to_owned(),
    )
}

fn admitted_remote(
    guest: AzureVmAdmittedGuest,
    cloud: AzureVmCloudIdentity,
    relationship: AdmittedRelationship,
    asked: Arc<Mutex<usize>>,
) -> Arc<AzureVmAdmittedRemote> {
    Arc::new(AzureVmAdmittedRemote::new(
        Arc::new(
            AzureVmRemoteAuthority::new(guest, cloud, Arc::new(relationship))
                .expect("the ARM audience is a bounded token"),
        ),
        Arc::new(FakeDelivery {
            sequence: 1,
            audience: AZURE_VM_CONTROL_AUDIENCE.to_owned(),
            asked,
        }),
    ))
}

/// A controller wired to the admitted authority.
fn admitted_controller(
    state: Arc<Mutex<FakeState>>,
    guest_uid: &str,
    generation: u64,
    requested_presentation: PresentationCapability,
    admitted_subscription: &str,
    controller_subscription: &str,
) -> (AzureVmController<FakeEffect>, Arc<Mutex<usize>>) {
    let (provider, settings) = config();
    let asked = Arc::new(Mutex::new(0));
    let remote = admitted_remote(
        admitted_guest(guest_uid, generation),
        azure_cloud(admitted_subscription, "resource-group"),
        AdmittedRelationship::live(),
        Arc::clone(&asked),
    );
    let controller = AzureVmController::new(
        provider,
        settings,
        FakeEffect {
            state: Arc::clone(&state),
        },
        credential(),
        None,
    )
    .unwrap()
    .with_cloud_identity(azure_cloud(controller_subscription, "resource-group"))
    .with_admitted_authority(remote)
    .with_requested_presentation(requested_presentation)
    .with_bootstrap_service(d2b_provider_guest_azure_virtual_machine::BootstrapService::from_state(
        d2b_provider_guest_azure_virtual_machine::BootstrapServiceState::Enrolled,
    ));
    (controller, asked)
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_ambiguous_provision_retries_onto_the_same_arm_operation() {
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        provision_failures: 1,
        ..FakeState::default()
    }));
    let (mut controller, _) = admitted_controller(
        Arc::clone(&state),
        ADMITTED_GUEST_UID,
        1,
        declared_presentation(),
        "subscription",
        "subscription",
    );

    // First pass: ARM is asked to provision, the machine is really created,
    // and the response never arrives.
    assert!(controller.reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1).await.is_err());
    {
        let state = state.lock().await;
        assert_eq!(state.provision_operation_ids.len(), 1);
        assert_eq!(state.stored_provisions, 1, "ARM really holds the machine");
    }

    // Second pass: the same deterministic operation id is presented, and the
    // retry reconciles the machine the lost response described instead of
    // provisioning a second one.
    controller.reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1).await.unwrap();
    assert_eq!(controller.phase(), AzureVmPhase::Ready);
    let state = state.lock().await;
    assert_eq!(state.stored_provisions, 1, "exactly one VM exists in ARM");
    assert_eq!(
        state.provision_operation_ids.len(),
        1,
        "the retry adopted the first attempt's machine instead of provisioning \
         a second one, because it observed it under the same deterministic \
         name before it was ever asked to provision"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_revoked_credential_prevents_new_remote_mutation() {
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        ..FakeState::default()
    }));
    let asked = Arc::new(Mutex::new(0));
    let (provider, settings) = config();
    let remote = admitted_remote(
        admitted_guest(ADMITTED_GUEST_UID, 1),
        azure_cloud("subscription", "resource-group"),
        AdmittedRelationship::revoked(),
        Arc::clone(&asked),
    );
    let mut controller = AzureVmController::new(
        provider,
        settings,
        FakeEffect {
            state: Arc::clone(&state),
        },
        credential(),
        None,
    )
    .unwrap()
    .with_cloud_identity(azure_cloud("subscription", "resource-group"))
    .with_admitted_authority(remote);

    assert_eq!(
        controller.reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1).await.unwrap_err(),
        AzureVmError::RemoteRefused
    );
    assert!(
        state.lock().await.calls.is_empty(),
        "a revoked credential must stop the call before ARM is read"
    );
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_guest_this_provider_did_not_admit_cannot_mutate_the_subscription() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let (mut controller, _) = admitted_controller(
        Arc::clone(&state),
        ADMITTED_GUEST_UID,
        1,
        declared_presentation(),
        "subscription",
        "subscription",
    );
    assert_eq!(
        controller
            .reconcile(ADMITTED_ZONE, OTHER_GUEST_UID, 1)
            .await
            .unwrap_err(),
        AzureVmError::RemoteRefused,
        "a reconcile naming a Guest this provider did not admit is refused"
    );
    assert!(state.lock().await.calls.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_controller_on_another_subscription_cannot_mutate_this_one() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let (mut controller, _) = admitted_controller(
        Arc::clone(&state),
        ADMITTED_GUEST_UID,
        1,
        declared_presentation(),
        "subscription",
        "another-subscription",
    );
    assert_eq!(
        controller
            .reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1)
            .await
            .unwrap_err(),
        AzureVmError::RemoteRefused
    );
    assert!(state.lock().await.calls.is_empty());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_unsupported_local_presentation_refuses_before_any_remote_call() {
    for requested in [
        PresentationCapability::FilesystemPresentation,
        PresentationCapability::NamespaceFirstServiceSource,
    ] {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let (mut controller, asked) = admitted_controller(
            Arc::clone(&state),
            ADMITTED_GUEST_UID,
            1,
            requested,
            "subscription",
            "subscription",
        );
        assert_eq!(
            controller
                .reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1)
                .await
                .unwrap_err(),
            AzureVmError::RemoteRefused,
            "{requested:?} is not something an Azure virtual machine can present"
        );
        assert!(
            state.lock().await.calls.is_empty(),
            "{requested:?} must not reach ARM"
        );
        assert_eq!(
            *asked.lock().await,
            0,
            "a refused presentation must not even ask the credential adapter"
        );
    }
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_supported_presentation_hands_arm_the_admitted_identity() {
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        ..FakeState::default()
    }));
    let (mut controller, _) = admitted_controller(
        Arc::clone(&state),
        ADMITTED_GUEST_UID,
        1,
        declared_presentation(),
        "subscription",
        "subscription",
    );
    controller.reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1).await.unwrap();

    // The effective-access evidence a remote backend can produce hermetically:
    // ARM was addressed under the one operation id the accepted Guest
    // identity derives, for that Guest, in that subscription. Whether Azure
    // then honours the machine is a live acceptance condition, not something
    // a fake can answer.
    let expected = {
        let remote = AzureVmRemoteAuthority::new(
            admitted_guest(ADMITTED_GUEST_UID, 1),
            azure_cloud("subscription", "resource-group"),
            Arc::new(AdmittedRelationship::live()),
        )
        .expect("authority");
        remote
            .reconciliation_key(AzureVmRemotePurpose::Provision)
            .operation_id()
            .to_owned()
    };
    assert_eq!(
        state.lock().await.provision_operation_ids,
        std::slice::from_ref(&expected)
    );

    let other = AzureVmRemoteAuthority::new(
        admitted_guest(OTHER_GUEST_UID, 1),
        azure_cloud("subscription", "resource-group"),
        Arc::new(AdmittedRelationship::live()),
    )
    .expect("authority")
    .reconciliation_key(AzureVmRemotePurpose::Provision)
    .operation_id()
    .to_owned();
    assert_ne!(expected, other, "two admitted Guests never share an operation id");
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn release_needs_every_precondition_to_be_confirmed() {
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    {
        let mut held = state.lock().await;
        held.polls = std::iter::repeat_n(LroStatus::Succeeded, 8).collect();
    }
    let (mut controller, _) = admitted_controller(
        Arc::clone(&state),
        ADMITTED_GUEST_UID,
        1,
        declared_presentation(),
        "subscription",
        "subscription",
    );

    controller.reconcile(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1).await.unwrap();
    let evidence = controller
        .release_evidence()
        .expect("an admitted controller records its identity");
    assert!(!evidence.is_terminal(), "a running VM is not a release");
    assert!(!evidence.vm_absent());

    for _ in 0..8 {
        state.lock().await.polls.push(LroStatus::Succeeded);
        if let Some(in_flight) = controller.recovery_state().in_flight_operation {
            controller.poll_operation(in_flight.operation).await.unwrap();
        }
        controller.finalize(ADMITTED_ZONE, ADMITTED_GUEST_UID, 1).await.unwrap();
        if controller.release_evidence().is_some_and(|value| value.is_terminal()) {
            break;
        }
    }
    let evidence = controller
        .release_evidence()
        .expect("a finalized Guest leaves evidence");
    assert!(
        evidence.is_terminal(),
        "release needs an absent VM, no bootstrap extension, finished child \
         cleanup, and a dropped finalizer: {evidence:?}"
    );
    assert!(evidence.vm_absent());
    assert!(evidence.bootstrap_extension_absent());
    assert!(evidence.child_cleanup_complete());
    assert!(evidence.finalizer_released());
    assert_eq!(evidence.guest_uid(), &graph_uid(ADMITTED_GUEST_UID));
    assert_eq!(evidence.generation(), 1);
}

#[test]
fn removed_configuration_and_operation_paths_are_refused_deterministically() {
    // A Guest settings blob carrying a field the removed model accepted is
    // refused by decode, not silently dropped.
    let mut settings = serde_json::to_value(config().1).expect("settings serialize");
    settings
        .as_object_mut()
        .expect("an object")
        .insert(
            "managedIdentityClientSecret".to_owned(),
            serde_json::Value::String("not-a-field-anymore".to_owned()),
        );
    assert!(
        serde_json::from_value::<AzureVmGuestSettings>(settings).is_err(),
        "a removed configuration field is refused, not ignored"
    );

    // The bootstrap PSK has exactly one delivery route left; the removed
    // local-delivery spellings do not decode.
    assert!(serde_json::from_str::<BootstrapPskDelivery>("\"user-data\"").is_err());
    assert!(serde_json::from_str::<BootstrapPskDelivery>("\"attached-disk\"").is_err());

    // Reads are the one purpose that must not present a delivery session, and
    // every mutating purpose must acquire one.
    assert_eq!(
        AzureVmRemotePurpose::Inspect.credential_method(),
        CredentialMethod::InspectMetadata
    );
    assert!(!AzureVmRemotePurpose::Inspect.mutates_remote_state());
    for purpose in [
        AzureVmRemotePurpose::Provision,
        AzureVmRemotePurpose::Bootstrap,
        AzureVmRemotePurpose::Delete,
    ] {
        assert!(purpose.mutates_remote_state(), "{purpose:?} mutates ARM");
        assert_eq!(
            purpose.credential_method(),
            CredentialMethod::AcquireToken
        );
        assert!(purpose.code().starts_with("azure-vm-remote-"));
    }
    assert_ne!(
        AzureVmRemotePurpose::Provision.operation_class(),
        AzureVmRemotePurpose::Delete.operation_class(),
        "two purposes never share a cloud name"
    );

    let spec = azure_vm_declaration();
    assert_eq!(spec.artifact_id().as_str(), AZURE_VM_ARTIFACT_ID);
    assert_eq!(
        spec.provider().to_canonical_string(),
        d2b_provider_guest_azure_virtual_machine::PROVIDER_REF
    );
    assert_eq!(declared_presentation(), PresentationCapability::None);
}

#[test]
fn a_recovered_record_is_fenced_on_the_admitted_identity_it_names() {
    let recovered = AdmittedRecoveryIdentity {
        zone: ADMITTED_ZONE.to_owned(),
        guest_uid: ADMITTED_GUEST_UID.to_owned(),
        provider_generation: 7,
        cloud_subscription_id: "subscription".to_owned(),
        cloud_resource_group: "resource-group".to_owned(),
    };
    assert_eq!(recovered.zone(), ADMITTED_ZONE);
    assert_eq!(recovered.guest_uid(), ADMITTED_GUEST_UID);
    assert_eq!(recovered.generation(), 7);
    assert_eq!(recovered.subscription_id(), "subscription");
    assert_eq!(recovered.resource_group(), "resource-group");
    assert!(recovered.matches(&recovered.clone()));

    // Anything that moved retires the record's authority, so a restart
    // cannot resume an in-flight ARM operation against a replaced Guest or a
    // reconfigured Provider.
    for replaced in [
        AdmittedRecoveryIdentity {
            guest_uid: OTHER_GUEST_UID.to_owned(),
            ..recovered.clone()
        },
        AdmittedRecoveryIdentity {
            provider_generation: 8,
            ..recovered.clone()
        },
        AdmittedRecoveryIdentity {
            zone: "other".to_owned(),
            ..recovered.clone()
        },
        AdmittedRecoveryIdentity {
            cloud_subscription_id: "other-subscription".to_owned(),
            ..recovered.clone()
        },
        AdmittedRecoveryIdentity {
            cloud_resource_group: "other-group".to_owned(),
            ..recovered.clone()
        },
    ] {
        assert!(
            !recovered.matches(&replaced),
            "a moved fence must not match: {replaced:?}"
        );
    }
}

#[test]
fn the_admitted_guest_refuses_a_graph_identity_that_is_not_this_provider() {
    assert_eq!(
        AzureVmAdmittedGuest::new(
            ZoneId::parse(ADMITTED_ZONE).unwrap(),
            graph_reference("Guest/workload"),
            graph_uid(ADMITTED_GUEST_UID),
            graph_reference("Provider/runtime-qemu-media"),
            1,
        )
        .unwrap_err(),
        AzureVmRemoteRefusal::ProviderIdentityMismatch
    );
    assert_eq!(
        AzureVmAdmittedGuest::new(
            ZoneId::parse(ADMITTED_ZONE).unwrap(),
            graph_reference("Guest/workload"),
            graph_uid(ADMITTED_GUEST_UID),
            graph_reference(d2b_provider_guest_azure_virtual_machine::PROVIDER_REF),
            0,
        )
        .unwrap_err(),
        AzureVmRemoteRefusal::GuestIdentityMismatch
    );
}
