use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts::v3::{ResourceRef, credential::OpaqueAzureRef};
use d2b_provider_runtime_azure_virtual_machine::{
    AzureEffectPort, AzureOperationHandle, AzureVmConfig, AzureVmController, AzureVmError,
    AzureVmGuestSettings, AzureVmHandle, AzureVmPhase, AzureVmReconcileOutcome, AzureVmState,
    BootstrapAdmission, BootstrapPsk, BootstrapPskDelivery, BootstrapService, DiskSku, LroStatus,
    PskExtensionPayload, TagDigest,
};

struct FakeState {
    state: AzureVmState,
    handle: Option<AzureVmHandle>,
    tags: Option<TagDigest>,
    calls: Vec<&'static str>,
    polls: Vec<LroStatus>,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            state: AzureVmState::Absent,
            handle: None,
            tags: None,
            calls: Vec::new(),
            polls: Vec::new(),
        }
    }
}

struct FakeEffect {
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl AzureEffectPort for FakeEffect {
    async fn start_vm_provision(
        &self,
        _: &AzureVmGuestSettings,
        _: &str,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push("provision");
        state.state = AzureVmState::Running;
        state.handle = Some(AzureVmHandle::from_core("opaque-vm").unwrap());
        state.tags = Some(TagDigest::from_core([8; 32]));
        Ok(AzureOperationHandle::from_core(b"provision").unwrap())
    }

    async fn poll_lro(&self, _: &AzureOperationHandle) -> Result<LroStatus, AzureVmError> {
        self.state
            .lock()
            .unwrap()
            .polls
            .pop()
            .ok_or(AzureVmError::Transient)
    }

    async fn get_vm_state(
        &self,
        _: &AzureVmGuestSettings,
    ) -> Result<(AzureVmState, Option<AzureVmHandle>, Option<TagDigest>), AzureVmError> {
        let state = self.state.lock().unwrap();
        Ok((state.state, state.handle.clone(), state.tags))
    }

    async fn put_vm_extension(
        &self,
        _: &AzureVmHandle,
        _: PskExtensionPayload,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        self.state.lock().unwrap().calls.push("extension");
        Ok(AzureOperationHandle::from_core(b"extension").unwrap())
    }

    async fn start_vm_resize(
        &self,
        _: &AzureVmHandle,
        _: &str,
        _: &str,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"resize").unwrap())
    }

    async fn start_vm_delete(
        &self,
        _: &AzureVmHandle,
        _: &str,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push("delete");
        state.state = AzureVmState::Absent;
        state.handle = None;
        state.tags = None;
        Ok(AzureOperationHandle::from_core(b"delete").unwrap())
    }

    async fn start_disk_attach(
        &self,
        _: &AzureVmHandle,
        _: &d2b_provider_runtime_azure_virtual_machine::DataDiskSpec,
        _: &str,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"attach").unwrap())
    }

    async fn start_disk_detach(
        &self,
        _: &AzureVmHandle,
        _: u8,
        _: &str,
    ) -> Result<AzureOperationHandle, AzureVmError> {
        Ok(AzureOperationHandle::from_core(b"detach").unwrap())
    }

    async fn update_vm_tags(
        &self,
        _: &AzureVmHandle,
        _: &[(String, String)],
        _: &str,
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

#[tokio::test]
async fn absent_vm_starts_non_blocking_provision() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect {
        state: Arc::clone(&state),
    });
    let mut controller = AzureVmController::new(
        provider,
        settings,
        effect,
        Some(BootstrapPsk::from_bytes(b"one-time").unwrap()),
    )
    .unwrap();
    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Progressing { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::Provisioning);
    assert_eq!(state.lock().unwrap().calls, ["provision"]);
}

#[tokio::test]
async fn restart_adopts_only_tagged_running_vm() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect {
        state: Arc::clone(&state),
    });
    let mut controller = AzureVmController::new(provider, settings, effect, None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    assert_eq!(
        controller.adopt().await.unwrap(),
        AzureVmReconcileOutcome::Converged
    );
    assert_eq!(controller.phase(), AzureVmPhase::Ready);
    assert!(!format!("{:?}", controller.status()).contains("opaque-vm"));
}

#[tokio::test]
async fn delete_keeps_finalizer_until_lro_completion() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        polls: vec![LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect { state });
    let mut controller = AzureVmController::new(provider, settings, effect, None)
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
    assert!(!controller.finalizer_installed());
    assert_eq!(controller.phase(), AzureVmPhase::Finalized);
}

#[tokio::test]
async fn running_vm_waits_for_authenticated_enrollment() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect { state });
    let mut controller = AzureVmController::new(provider, settings, effect, None).unwrap();
    assert!(matches!(
        controller.reconcile("zone", "guest", 1).await.unwrap(),
        AzureVmReconcileOutcome::Retry { .. }
    ));
    assert_eq!(controller.phase(), AzureVmPhase::Bootstrapping);
    assert!(controller.status().identity_digest().is_none());
}

#[tokio::test]
async fn foreign_tags_are_not_adopted() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(TagDigest::from_core([9; 32])),
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect { state });
    let mut controller = AzureVmController::new(provider, settings, effect, None)
        .unwrap()
        .with_bootstrap_service(enrolled_service());
    assert_eq!(
        controller.adopt().await.unwrap_err(),
        AzureVmError::ArmResourceConflict
    );
    assert_eq!(controller.phase(), AzureVmPhase::Failed);
}

#[tokio::test]
async fn restart_finalization_reobserves_before_clearing_finalizer() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Running,
        handle: Some(AzureVmHandle::from_core("opaque-vm").unwrap()),
        tags: Some(expected_tag_digest()),
        polls: vec![LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect {
        state: Arc::clone(&state),
    });
    let mut controller = AzureVmController::new(provider, settings, effect, None)
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
    assert!(!controller.finalizer_installed());
}

#[tokio::test]
async fn provisioning_lro_delivers_psk_before_bootstrap_phase() {
    let (provider, settings) = config();
    let state = Arc::new(Mutex::new(FakeState {
        state: AzureVmState::Absent,
        polls: vec![LroStatus::Succeeded, LroStatus::Succeeded],
        ..FakeState::default()
    }));
    let effect = Arc::new(FakeEffect {
        state: Arc::clone(&state),
    });
    let mut controller = AzureVmController::new(
        provider,
        settings,
        effect,
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
    assert_eq!(state.lock().unwrap().calls, ["provision", "extension"]);
}
