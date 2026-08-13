use d2b_contracts::v3::ResourceRef;
use d2b_provider_audio_pipewire::{
    AudioBindingSpec, AudioServiceRole, AudioServiceSpec, ProviderExtension,
    validate_audio_binding, validate_audio_service,
};

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
