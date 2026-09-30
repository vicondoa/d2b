use std::num::NonZeroUsize;

use d2b_contracts_resource::v3::{
    BindingSlot, EndpointAttachmentKind, EndpointBindingRequest, ResourceGeneration, ResourceRef,
    ResourceUid, ZoneRevision, execution_policy::BoundedToken,
};
use d2b_provider_audio_pipewire::{
    AudioAuthorityError, AudioChannel, AudioLeaseId, AudioSessionOrigin, ChannelSession,
    MicDecision, MicrophoneArbiter, SpeakerMixer,
};

/// The exact endpoint request one channel's relationship names.
fn request(channel: AudioChannel, endpoint: &str) -> EndpointBindingRequest {
    EndpointBindingRequest::new(
        ResourceRef::parse(endpoint).expect("endpoint"),
        ResourceRef::parse("Guest/workstation").expect("guest"),
        BindingSlot::parse(channel.binding_slot()).expect("slot"),
        EndpointAttachmentKind::Connect,
        BoundedToken::parse(channel.declared_purpose()).expect("purpose"),
    )
    .expect("endpoint binding request")
}

fn fence(byte: u8, generation: u64) -> d2b_provider_audio_pipewire::AudioBindingFence {
    d2b_provider_audio_pipewire::AudioBindingFence::new(
        ResourceUid::from_bytes(&[byte; 16]).expect("uuid"),
        ResourceGeneration::new(generation).expect("generation"),
        ZoneRevision::new(7),
    )
}

/// The admitted relationship one channel's bookkeeping rides.
fn channel(
    target: AudioChannel,
    endpoint: &str,
    byte: u8,
    generation: u64,
) -> ChannelSession {
    let admitted = fence(byte, generation);
    ChannelSession::admit(
        target,
        &d2b_provider_audio_pipewire::AdmittedAudioSession::new(
            target,
            request(target, endpoint),
            admitted,
            AudioSessionOrigin::Owner,
        )
        .expect("admitted session"),
    )
    .expect("channel session")
}

fn speaker(byte: u8, generation: u64) -> ChannelSession {
    channel(AudioChannel::Speaker, "Endpoint/audio-speaker", byte, generation)
}

fn microphone(byte: u8, generation: u64) -> ChannelSession {
    channel(
        AudioChannel::Microphone,
        "Endpoint/audio-microphone",
        byte,
        generation,
    )
}

#[test]
fn microphone_is_exclusive_and_fair_with_bounded_queue() {
    let mut arbiter = MicrophoneArbiter::new(NonZeroUsize::new(2).unwrap());
    let session = microphone(0x11, 1);
    assert_eq!(
        arbiter.request(AudioLeaseId::new(1), &session),
        Ok(MicDecision::Granted)
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(2), &session),
        Ok(MicDecision::Queued)
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(3), &session),
        Ok(MicDecision::Queued)
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(4), &session),
        Ok(MicDecision::QueueFull)
    );
    assert!(arbiter.release(AudioLeaseId::new(1)));
    assert_eq!(arbiter.next_lease(), Some(AudioLeaseId::new(2)));
}

#[test]
fn queued_microphone_requests_remain_queued_until_handoff() {
    let mut arbiter = MicrophoneArbiter::new(NonZeroUsize::new(1).unwrap());
    let session = microphone(0x11, 1);
    assert_eq!(
        arbiter.request(AudioLeaseId::new(1), &session),
        Ok(MicDecision::Granted)
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(2), &session),
        Ok(MicDecision::Queued)
    );
    assert_eq!(
        arbiter.request(AudioLeaseId::new(2), &session),
        Ok(MicDecision::Queued)
    );
    assert_eq!(arbiter.pending_count(), 1);
}

#[test]
fn speaker_mixer_keeps_grants_independent() {
    let mut mixer = SpeakerMixer::new(NonZeroUsize::new(2).unwrap());
    let session = speaker(0x11, 1);
    mixer
        .set_level(AudioLeaseId::new(1), 80, &session)
        .expect("level 80");
    mixer
        .set_level(AudioLeaseId::new(2), 20, &session)
        .expect("level 20");
    assert_eq!(mixer.mix_level(), 100);
}

#[test]
fn speaker_mixer_mix_level_is_capped_at_100() {
    let mut mixer = SpeakerMixer::new(NonZeroUsize::new(3).unwrap());
    let session = speaker(0x11, 1);
    for (lease, level) in [(1, 80), (2, 80), (3, 60)] {
        mixer
            .set_level(AudioLeaseId::new(lease), level, &session)
            .expect("bounded level");
    }
    assert_eq!(mixer.mix_level(), 100);
}

#[test]
fn a_microphone_relationship_cannot_take_a_speaker_grant() {
    let mut mixer = SpeakerMixer::new(NonZeroUsize::new(2).unwrap());
    assert_eq!(
        mixer.grant(AudioLeaseId::new(1), &microphone(0x22, 1)),
        Err(AudioAuthorityError::SessionChannelMismatch),
        "the microphone's admitted relationship is not a speaker grant"
    );
    assert!(
        !mixer.has_grant(AudioLeaseId::new(1)),
        "the refused grant left no speaker state behind"
    );

    let mut arbiter = MicrophoneArbiter::new(NonZeroUsize::new(2).unwrap());
    assert_eq!(
        arbiter.request(AudioLeaseId::new(1), &speaker(0x11, 1)),
        Err(AudioAuthorityError::SessionChannelMismatch),
        "the speaker's admitted relationship is not a capture grant"
    );
    assert_eq!(arbiter.active(), None);
}

#[test]
fn a_level_cannot_ride_a_relationship_the_grant_did_not_take() {
    let mut mixer = SpeakerMixer::new(NonZeroUsize::new(2).unwrap());
    let lease = AudioLeaseId::new(1);
    assert!(mixer.grant(lease, &speaker(0x11, 1)).expect("speaker grant"));
    assert_eq!(
        mixer.set_level(lease, 60, &speaker(0x33, 1)),
        Err(AudioAuthorityError::SessionNotCurrent),
        "a different admitted relationship cannot continue this consumer's grant"
    );
    assert_eq!(mixer.level(lease), None);
    assert_eq!(
        mixer.set_level(lease, 60, &speaker(0x11, 2)),
        Err(AudioAuthorityError::SessionNotCurrent),
        "a re-committed relationship is not the one the grant was taken under"
    );
    assert_eq!(
        mixer.set_level(lease, 60, &speaker(0x11, 1)),
        Ok(()),
        "the relationship the grant was taken under still sets its level"
    );
    assert_eq!(mixer.level(lease), Some(60));
}

#[test]
fn an_unadmitted_pass_cannot_continue_an_admitted_grant() {
    let mut mixer = SpeakerMixer::new(NonZeroUsize::new(2).unwrap());
    let lease = AudioLeaseId::new(1);
    assert!(mixer.grant(lease, &speaker(0x11, 1)).expect("speaker grant"));
    assert_eq!(
        mixer.set_level(lease, 30, &ChannelSession::UNADMITTED),
        Err(AudioAuthorityError::SessionNotCurrent),
        "an unadmitted pass cannot continue an admitted grant"
    );
    let mut arbiter = MicrophoneArbiter::new(NonZeroUsize::new(2).unwrap());
    assert_eq!(
        arbiter.request(lease, &microphone(0x11, 1)),
        Ok(MicDecision::Granted)
    );
    assert_eq!(
        arbiter.request(lease, &ChannelSession::UNADMITTED),
        Err(AudioAuthorityError::SessionNotCurrent),
        "an unadmitted pass cannot continue an admitted capture grant"
    );
    assert_eq!(arbiter.active(), Some(lease));
}

#[test]
fn an_imported_projection_cannot_own_a_channel_grant() {
    let projected = d2b_provider_audio_pipewire::AdmittedAudioSession::new(
        AudioChannel::Speaker,
        request(AudioChannel::Speaker, "Endpoint/audio-speaker"),
        fence(0x33, 1),
        AudioSessionOrigin::ImportedProjection,
    )
    .expect("admitted projection session");
    assert_eq!(
        ChannelSession::admit(AudioChannel::Speaker, &projected),
        Err(AudioAuthorityError::ImportedProjectionNotOwner),
        "an imported Service projection consumes a session; it owns none"
    );
}

#[test]
fn a_lease_is_minted_from_the_relationship_it_represents() {
    let uid = ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("uid");
    let first = ResourceGeneration::new(1).expect("generation");
    let second = ResourceGeneration::new(2).expect("generation");
    assert_eq!(
        AudioLeaseId::for_binding(&uid, first),
        AudioLeaseId::for_binding(&uid, first),
        "a restart over the same committed relationship resumes the same lease"
    );
    assert_ne!(
        AudioLeaseId::for_binding(&uid, first),
        AudioLeaseId::for_binding(&uid, second),
        "a re-committed relationship is a different lease and inherits nothing"
    );
    let other = ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("uid");
    assert_ne!(
        AudioLeaseId::for_binding(&uid, first),
        AudioLeaseId::for_binding(&other, first),
        "two relationships are never one lease"
    );
}
