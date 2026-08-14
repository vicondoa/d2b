use d2b_contracts::v3::ResourceRef;
use d2b_provider_audio_pipewire::{
    AudioBindingSpec, AudioServiceRole, AudioServiceSpec, ProviderExtension,
    validate_audio_binding, validate_audio_service,
};
use serde_json::json;

#[test]
fn owner_service_and_binding_are_provider_neutral_at_the_base() {
    let owner = AudioServiceSpec::owner(
        ResourceRef::parse("Endpoint/audio-authority").unwrap(),
        "zone-a",
    )
    .unwrap();
    assert_eq!(owner.service_role, AudioServiceRole::Owner);
    let binding = AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "zone-a",
    )
    .unwrap();
    assert!(validate_audio_service(&owner).is_ok());
    assert!(validate_audio_binding(&binding).is_ok());
    assert!(
        d2b_provider_audio_pipewire::validate_audio_binding_in_zone(&binding, "zone-a").is_ok()
    );
}

#[test]
fn resource_specs_match_frozen_audio_wire_shape() {
    let owner = AudioServiceSpec::owner(
        ResourceRef::parse("Endpoint/audio-authority").unwrap(),
        "zone-a",
    )
    .unwrap();
    let owner_json = serde_json::to_value(&owner).unwrap();
    assert_eq!(
        owner_json["operations"],
        json!(["playback", "capture"]),
        "AudioService operations must be present on the wire"
    );
    assert!(
        owner_json.get("zone").is_none(),
        "Zone belongs to resource metadata, not AudioService.spec"
    );
    let decoded_owner: AudioServiceSpec = serde_json::from_value(owner_json).unwrap();
    assert!(validate_audio_service(&decoded_owner).is_ok());

    let binding = AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "zone-a",
    )
    .unwrap();
    let binding_json = serde_json::to_value(&binding).unwrap();
    assert!(
        binding_json.get("zone").is_none(),
        "Zone belongs to resource metadata, not AudioBinding.spec"
    );
    let decoded_binding: AudioBindingSpec = serde_json::from_value(binding_json).unwrap();
    assert!(validate_audio_binding(&decoded_binding).is_ok());
}

#[test]
fn projection_and_cross_zone_or_provider_fields_fail_closed() {
    let projection = AudioServiceSpec::projection("zone-b").unwrap();
    assert!(validate_audio_service(&projection).is_ok());
    let foreign = AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/remote").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "zone-a",
    )
    .unwrap()
    .with_provider_extension(ProviderExtension::new("node-id"));
    assert!(validate_audio_binding(&foreign).is_err());
    let cross_zone = AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/remote").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "zone-b",
    )
    .unwrap();
    assert_eq!(
        d2b_provider_audio_pipewire::validate_audio_binding_in_zone(&cross_zone, "zone-a"),
        Err(d2b_provider_audio_pipewire::AudioAdmissionError::CrossZone)
    );
}
