use d2b_contracts::v3::device::DeviceArbitration;
use d2b_provider_device_gpu::{GpuSettings, GpuSettingsError};

#[test]
fn settings_round_trip_and_unknown_fields_are_closed() {
    let settings = GpuSettings::default();
    let json = serde_json::to_string(&settings).unwrap();
    let decoded: GpuSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, settings);
    assert!(serde_json::from_str::<GpuSettings>(r#"{"artifactId":"x"}"#).is_err());
    let shared = GpuSettings {
        render_node_only: true,
        video_sidecar: true,
        ..GpuSettings::default()
    };
    assert_eq!(
        shared.validate(DeviceArbitration::Shared),
        Err(GpuSettingsError::VideoRequiresFullGpu)
    );
}
