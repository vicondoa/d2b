use d2b_provider_audio_pipewire::{
    AudioGrant, AudioMediator, AudioMediatorError, AudioReadiness, FakeAudioMediator,
    GuestAudioReadiness, HostAudioReadiness, LevelPercent,
};

#[test]
fn host_and_guest_readiness_remain_distinct() {
    let mediator = FakeAudioMediator::ready();
    assert_eq!(mediator.host_readiness(), HostAudioReadiness::Ready);
    assert_eq!(mediator.guest_readiness(), GuestAudioReadiness::Ready);
}

#[test]
fn projection_cannot_open_pipewire_and_failed_set_preserves_state() {
    let mut mediator = FakeAudioMediator::projection();
    assert_eq!(
        mediator.set_grant(AudioGrant::On),
        Err(AudioMediatorError::ProjectionCannotOpenPipewire)
    );
    assert_eq!(
        mediator.set_level(LevelPercent::new(80).unwrap()),
        Err(AudioMediatorError::ProjectionCannotOpenPipewire)
    );
    assert_eq!(mediator.readiness(), AudioReadiness::Unavailable);
}
