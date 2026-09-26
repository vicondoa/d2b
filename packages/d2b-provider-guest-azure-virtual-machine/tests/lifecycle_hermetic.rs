use std::sync::Arc;
use tokio::sync::Mutex;

use async_trait::async_trait;
use d2b_contracts::{OpaqueAzureRef, ResourceRef};
use d2b_provider_guest_azure_virtual_machine::{
    AzureAccessToken, AzureCredentialPort, AzureEffectPort, AzureOperationHandle, AzureVmConfig,
    AzureVmController, AzureVmError, AzureVmGuestSettings, AzureVmHandle, AzureVmPhase,
    AzureVmReconcileOutcome, AzureVmRecoveryState, AzureVmState, AzureVmUpdate, BootstrapAdmission,
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
        _: &str,
        _: &AzureAccessToken,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().await;
        state.calls.push("provision");
        state.state = AzureVmState::Running;
        state.handle = Some(AzureVmHandle::from_core("opaque-vm").unwrap());
        state.tags = Some(TagDigest::from_tags(&[(
            "owner".to_owned(),
            "d2b".to_owned(),
        )]));
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

#[test]
fn azure_vm_update_resize_round_trips_with_the_plain_string_wire_shape() {
    let update = AzureVmUpdate::Resize {
        size: OpaqueAzureRef::parse("standard-d8").unwrap(),
    };
    let encoded = serde_json::to_value(&update).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({ "resize": { "size": "standard-d8" } })
    );
    assert_eq!(
        serde_json::from_value::<AzureVmUpdate>(encoded).unwrap(),
        update
    );
    assert!(
        serde_json::from_str::<AzureVmUpdate>(r#"{"resize":{"size":""}}"#).is_err(),
        "the size SKU is validated at the deserialization boundary"
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
async fn restart_adopts_only_tagged_running_vm() {
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
        controller.adopt().await.unwrap(),
        AzureVmReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AzureVmPhase::Ready);
    assert!(!format!("{:?}", controller.status()).contains("opaque-vm"));
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
    controller.adopt().await.unwrap();
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
    assert!(controller.status().identity_digest().is_none());
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn ready_vm_accepts_typed_resize_and_commits_after_lro() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        polls: vec![LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = FakeEffect {
        state: Arc::clone(&state),
    };
    let mut controller = AzureVmController::new(provider, settings, effect, credential(), None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    controller.adopt().await.unwrap();
    assert!(matches!(
        controller
            .update(
                "zone",
                "guest",
                1,
                AzureVmUpdate::Resize {
                    size: OpaqueAzureRef::parse("standard-d8").unwrap(),
                },
            )
            .await
            .unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::Reconfiguring);
    assert_eq!(
        controller
            .poll_operation(AzureOperationHandle::from_core(b"resize").unwrap())
            .await
            .unwrap(),
        AzureVmReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AzureVmPhase::Ready);
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn failed_update_lro_honors_pending_delete_intent() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
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
    controller.adopt().await.unwrap();
    controller
        .update(
            "zone",
            "guest",
            1,
            AzureVmUpdate::Resize {
                size: OpaqueAzureRef::parse("standard-d8").unwrap(),
            },
        )
        .await
        .unwrap();
    controller.finalize("zone", "guest", 2).await.unwrap();
    assert_eq!(
        controller
            .poll_operation(AzureOperationHandle::from_core(b"resize").unwrap())
            .await
            .unwrap(),
        AzureVmReconcileOutcome::Progressing { after_ms: 1_000 }
    );
    assert_eq!(controller.phase(), AzureVmPhase::Deleting);
    assert!(controller.finalizer_installed());
    assert!(controller.recovery_state().pending_update.is_none());
    assert_eq!(state.lock().await.calls, ["delete"]);
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
            pending_update: None,
            bootstrap_service_state: BootstrapService::default().state(),
            bootstrap_extension_present: false,
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
async fn foreign_tags_are_not_adopted() {
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
        controller.adopt().await.unwrap_err(),
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
        pending_update: None,
        bootstrap_service_state: BootstrapService::default().state(),
        bootstrap_extension_present: true,
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
