use d2b_contracts::v3::device::DeviceArbitration;
use d2b_provider_device_gpu::{GpuEffectToken, GpuEffectTokenSet, GpuSettings, GpuSettingsError};

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

#[test]
fn nvidia_decode_requires_the_video_sidecar() {
    let settings = GpuSettings {
        video_nvidia_decode: true,
        ..GpuSettings::default()
    };
    assert_eq!(
        settings.validate(DeviceArbitration::Exclusive),
        Err(GpuSettingsError::NvidiaDecodeRequiresVideoSidecar)
    );
}

#[test]
fn context_types_are_unique() {
    let settings = GpuSettings {
        context_types: vec![
            d2b_provider_device_gpu::ContextType::Virgl,
            d2b_provider_device_gpu::ContextType::Virgl,
        ],
        ..GpuSettings::default()
    };
    assert_eq!(
        settings.validate(DeviceArbitration::Exclusive),
        Err(GpuSettingsError::DuplicateContextType)
    );
}

#[test]
fn effect_token_set_is_nonempty_and_bounded() {
    assert!(GpuEffectTokenSet::from_core(Vec::new()).is_err());

    let tokens = GpuEffectTokenSet::from_core(vec![GpuEffectToken::from_core([2; 32])]).unwrap();
    assert!(!tokens.is_empty());
    assert_eq!(tokens.len(), 1);

    let too_many = (0..9)
        .map(|value| GpuEffectToken::from_core([value; 32]))
        .collect();
    assert!(GpuEffectTokenSet::from_core(too_many).is_err());
}
