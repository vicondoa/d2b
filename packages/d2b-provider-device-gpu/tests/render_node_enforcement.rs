use d2b_contracts::v3::device::DeviceArbitration;
use d2b_provider_device_gpu::{GpuSettings, GpuSettingsError};

#[test]
fn shared_device_requires_render_node_only() {
    assert_eq!(
        GpuSettings::default().validate(DeviceArbitration::Shared),
        Err(GpuSettingsError::SharedRequiresRenderNodeOnly)
    );
    let settings = GpuSettings {
        render_node_only: true,
        ..GpuSettings::default()
    };
    assert!(settings.validate(DeviceArbitration::Shared).is_ok());
}
