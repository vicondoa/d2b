use d2b_contracts::v3::ResourceRef;
use d2b_provider_audio_pipewire::{
    AudioBindingController, AudioBindingPhase, AudioGrant, AudioLeaseId, AudioMediatorError,
    FakeAudioMediator, validate_audio_binding,
};

fn binding() -> d2b_provider_audio_pipewire::AudioBindingSpec {
    d2b_provider_audio_pipewire::AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "zone-a",
    )
    .unwrap()
}

#[test]
fn binding_controller_keeps_host_and_guest_readiness_separate() {
    let mediator = FakeAudioMediator::ready();
    let mut controller = AudioBindingController::new(mediator);
    let mut requested = binding();
    requested.grants.speaker = AudioGrant::On;
    let result = controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    assert_eq!(result.status.phase, AudioBindingPhase::Ready);
    assert!(result.host_effect_applied);
    assert_eq!(
        result.status.host_readiness,
        d2b_provider_audio_pipewire::HostAudioReadiness::Ready
    );
}

#[test]
fn projection_failure_does_not_report_ready_or_leak_a_handle() {
    let mediator = FakeAudioMediator::projection();
    let mut controller = AudioBindingController::new(mediator);
    let mut requested = binding();
    requested.grants.mic = d2b_provider_audio_pipewire::AudioGrant::On;
    let error = controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap_err();
    assert_eq!(
        error,
        d2b_provider_audio_pipewire::AudioControllerError::Mediator(
            AudioMediatorError::ProjectionCannotOpenPipewire
        )
    );
    assert!(validate_audio_binding(&requested).is_ok());
}

#[test]
fn controller_rejects_cross_zone_service_admission() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let error = controller
        .reconcile(&binding(), "zone-b", AudioLeaseId::new(1))
        .unwrap_err();
    assert_eq!(
        error,
        d2b_provider_audio_pipewire::AudioControllerError::Admission
    );
}

#[test]
fn queued_microphone_binding_is_not_ready() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    assert_eq!(
        controller
            .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
            .unwrap()
            .status
            .microphone,
        Some(d2b_provider_audio_pipewire::MicDecision::Granted)
    );
    let result = controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();
    assert_eq!(
        result.status.microphone,
        Some(d2b_provider_audio_pipewire::MicDecision::Queued)
    );
    assert_eq!(result.status.phase, AudioBindingPhase::Pending);
}

#[test]
fn speaker_admission_rejects_before_mutating_mediator() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.speaker_level =
        Some(d2b_provider_audio_pipewire::LevelPercent::new(25).expect("bounded test level"));
    for lease in 1..=64 {
        controller
            .reconcile(&requested, "zone-a", AudioLeaseId::new(lease))
            .unwrap();
    }
    let last_level = controller.mediator().level();
    assert_eq!(
        controller
            .reconcile(&requested, "zone-a", AudioLeaseId::new(65))
            .unwrap_err(),
        d2b_provider_audio_pipewire::AudioControllerError::Admission
    );
    assert_eq!(controller.mediator().level(), last_level);
}
