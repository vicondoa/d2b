use d2b_provider_audio_pipewire::{AudioLeaseId, MicDecision, MicrophoneArbiter, SpeakerMixer};

#[test]
fn microphone_is_exclusive_and_fair_with_bounded_queue() {
    let mut arbiter = MicrophoneArbiter::new(2);
    assert_eq!(
        arbiter.request(AudioLeaseId::new(1), "zone-a"),
        MicDecision::Granted
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(2), "zone-b"),
        MicDecision::Queued
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(3), "zone-c"),
        MicDecision::Queued
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(4), "zone-d"),
        MicDecision::QueueFull
    );
    assert!(arbiter.release(AudioLeaseId::new(1)));
    assert_eq!(arbiter.next_lease(), Some(AudioLeaseId::new(2)));
}

#[test]
fn speaker_mixer_keeps_grants_independent() {
    let mut mixer = SpeakerMixer::new(2);
    mixer.set_level(AudioLeaseId::new(1), 80).unwrap();
    mixer.set_level(AudioLeaseId::new(2), 20).unwrap();
    assert_eq!(mixer.mix_level(), 100);
}
