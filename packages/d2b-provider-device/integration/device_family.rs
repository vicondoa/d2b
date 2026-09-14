//! Integration scenario: the `Device` type's declaration as the plane sees it.
//!
//! The hermetic suite in `tests/device_family.rs` drives the rows; this
//! scenario checks the shape the plane registers: one descriptor for the
//! `Device` ResourceType, one factory that serves that type, and four
//! declared rows whose Provider identities are the realizer crates' own
//! constants. It needs no daemon, broker, or host hardware.

use std::sync::Arc;

use d2b_contracts_resource::v3::ControllerGeneration;
use d2b_provider_device::{
    DEVICE_REGISTRATIONS, DeviceDriverArgs, DeviceDriverEffects, DeviceResourceState,
    device_descriptor,
};
use d2b_provider_toolkit::{
    SharedProviderEffectOutcome, SharedProviderEffectError, SharedProviderEffectPhase,
    SharedProviderEffectRequest, SharedProviderFinalize,
};

struct UnavailableEffects;

#[async_trait::async_trait]
impl DeviceDriverEffects for UnavailableEffects {
    async fn reconcile_device(
        &self,
        _component: d2b_provider_device::DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        Ok(SharedProviderEffectOutcome::phase(
            SharedProviderEffectPhase::Pending,
        ))
    }

    async fn finalize_device(
        &self,
        _component: d2b_provider_device::DeviceComponent,
        _request: &SharedProviderEffectRequest<'_>,
        _state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        Ok(SharedProviderFinalize::Complete)
    }
}

#[test]
fn the_device_type_registers_one_driver_over_four_provider_rows() {
    let descriptor = device_descriptor(DeviceDriverArgs {
        zone: "integration".to_owned(),
        controller_generation: ControllerGeneration::new(1).expect("generation"),
        effects: Arc::new(UnavailableEffects),
    });
    let registered = descriptor
        .factory
        .resource_types()
        .iter()
        .map(|resource_type| resource_type.as_str().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        registered,
        vec!["Device".to_owned()],
        "one driver per resource type: the Device type is declared once"
    );
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        "Device"
    );
    assert_eq!(
        DEVICE_REGISTRATIONS
            .iter()
            .filter(|row| row.resource_type == "Device")
            .count(),
        4,
        "the four hardware Providers are rows of this declaration"
    );
}
