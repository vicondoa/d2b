//! The `Device` type's driver over a recording Provider port.
//!
//! The Device type is served by four hardware Providers and the plane keys one
//! driver per ResourceType, so these tests pin the two properties that keeps
//! true: the declaration serves exactly the `Device` type over the four
//! realizer crates' exported identities, and a row is driven by the component
//! its Provider reference selects.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ControllerGeneration;
use d2b_provider_device::{
    DEVICE_REGISTRATIONS, DEVICE_RESYNC, DEVICE_TYPE_NAME, DeviceComponent, DeviceDriverArgs,
    device_descriptor,
};
use d2b_provider_device::test_support::RecordingEffects;
use d2b_resource_runtime::context::{
    ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
    WatchRegistration,
};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{
    ResourceKey, ResourceProvenance, StoredDesiredResource,
};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use d2b_resource_runtime::target::TargetHandle;
use serde_json::json;

struct DeadManager;

#[async_trait]
impl ManagerEndpoint for DeadManager {
    async fn ensure_child(
        &self,
        _parent: &ResourceKey,
        _child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        Err(ResourceError::ManagerRpc("manager unavailable".to_owned()))
    }

    async fn get(&self, _key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        Ok(None)
    }

    async fn view(
        &self,
        _key: &ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
        Ok(None)
    }

    async fn delete(&self, _key: &ResourceKey) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn list_owned(&self, _owner_uid: [u8; 16]) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        Ok(Vec::new())
    }

    async fn register_watch(
        &self,
        _subscriber: &ResourceKey,
        _registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        Ok(WatchId(1))
    }

    async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingRequeue;

impl RequeueScheduler for RecordingRequeue {
    fn schedule(&self, _key: ResourceKey, _after: Duration) -> RequeueId {
        RequeueId(1)
    }

    fn cancel(&self, _id: RequeueId) {}
}

fn descriptor(effects: Arc<RecordingEffects>) -> d2b_resource_types::DriverDescriptor {
    device_descriptor(DeviceDriverArgs {
        zone: "dev".to_owned(),
        controller_generation: ControllerGeneration::new(1).expect("generation"),
        effects,
    })
}

fn context(
    descriptor: &d2b_resource_types::DriverDescriptor,
    provider_ref: &str,
) -> ResourceContext {
    let row = StoredDesiredResource {
        key: ResourceKey::new("dev", DEVICE_TYPE_NAME, "tpm-0"),
        uid: [0x42; 16],
        generation: 3,
        owner_uid: None,
        provenance: ResourceProvenance::Api,
        deleting: false,
        spec: serde_json::to_vec(&json!({ "providerRef": provider_ref })).expect("spec"),
        metadata: Vec::new(),
        created_at: 1_725_000_000,
    };
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    ResourceContext::new(
        row,
        TargetHandle::Host,
        descriptor.decoder.clone(),
        Arc::new(DeadManager),
        Arc::new(RecordingRequeue),
        effects_tx,
        notify_tx,
    )
}

/// The declaration serves exactly the `Device` type, over the four realizer
/// crates' exported Provider identities.
#[test]
fn descriptor_declares_the_device_type_over_four_providers() {
    let descriptor = descriptor(Arc::new(RecordingEffects::default()));
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        DEVICE_TYPE_NAME
    );
    let types = descriptor
        .factory
        .resource_types()
        .iter()
        .map(|resource_type| resource_type.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(types, vec![DEVICE_TYPE_NAME.to_owned()]);
    let providers = DEVICE_REGISTRATIONS
        .iter()
        .map(|row| row.provider_ref)
        .collect::<Vec<_>>();
    assert_eq!(
        providers,
        vec![
            d2b_provider_device_tpm::PROVIDER_REF,
            d2b_provider_device_usbip::PROVIDER_REF,
            d2b_provider_device_security_key::PROVIDER_REF,
            d2b_provider_device_gpu::PROVIDER_REF,
        ]
    );
    for row in &DEVICE_REGISTRATIONS {
        assert_eq!(row.resource_type, DEVICE_TYPE_NAME);
        assert_eq!(row.resync, DEVICE_RESYNC);
    }
}

/// A row is driven by the component its Provider reference selects, and a
/// Provider outside the four is terminal.
#[tokio::test]
async fn a_row_runs_the_effect_of_the_provider_its_spec_names() {
    let effects = Arc::new(RecordingEffects::default());
    let descriptor = descriptor(Arc::clone(&effects));
    let mut ctx = context(&descriptor, d2b_provider_device_tpm::PROVIDER_REF);
    let mut driver = descriptor.factory.create(ctx.key()).await;
    driver.validate(&mut ctx).await.expect("tpm row validates");
    driver.reconcile(&mut ctx).await.expect("tpm row reconciles");
    assert_eq!(
        *effects.reconciled.lock(),
        vec![DeviceComponent::Tpm],
        "the tpm Provider reference selects the tpm component"
    );

    let mut ctx = context(&descriptor, "Provider/volume-local");
    let mut driver = descriptor.factory.create(ctx.key()).await;
    let failure = driver.validate(&mut ctx).await.expect_err("must refuse");
    assert_eq!(
        failure,
        d2b_resource_runtime::error::DriverFailure::terminal(
            d2b_resource_runtime::error::DriverOp::Validate
        )
    );
}
