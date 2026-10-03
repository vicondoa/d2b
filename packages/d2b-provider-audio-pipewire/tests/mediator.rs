use d2b_contracts_resource::v3::{
    BindingSlot, EndpointAttachmentKind, EndpointBindingRequest, ResourceGeneration, ResourceRef,
    ResourceUid, ZoneRevision, execution_policy::BoundedToken,
};
use d2b_provider_audio_pipewire::{
    AUDIO_DECLARED_METHODS, AdmittedAudioMediator, AdmittedAudioSession, AudioBindingFence,
    AudioChannel, AudioEffectKind, AudioGrant, AudioMediator, AudioMediatorError, AudioReadiness,
    AudioSessionOrigin, AudioSessionPlan, FakeAudioMediator, GuestAudioReadiness,
    HostAudioReadiness, LevelPercent, audio_declared_method, service_backing_endpoint,
    service_declares_channel,
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

fn fence(byte: u8, generation: u64) -> AudioBindingFence {
    AudioBindingFence::new(
        ResourceUid::from_bytes(&[byte; 16]).expect("uuid"),
        ResourceGeneration::new(generation).expect("generation"),
        ZoneRevision::new(7),
    )
}

fn session(
    channel: AudioChannel,
    endpoint: &str,
    fence: &AudioBindingFence,
    origin: AudioSessionOrigin,
) -> AdmittedAudioSession {
    AdmittedAudioSession::new(channel, request(channel, endpoint), fence.clone(), origin)
        .expect("admitted session")
}

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
    assert_eq!(mediator.grant(), AudioGrant::Off);
    assert_eq!(mediator.level(), None);
}

#[test]
fn the_family_declares_one_method_per_channel_effect() {
    let names: Vec<&str> = AUDIO_DECLARED_METHODS
        .iter()
        .map(|method| method.name())
        .collect();
    assert_eq!(
        names,
        vec![
            "set-speaker-grant",
            "set-speaker-level",
            "set-microphone-grant",
            "set-microphone-gain",
        ],
        "one declared method per channel effect, in channel declaration order"
    );
    for method in AUDIO_DECLARED_METHODS {
        assert_eq!(
            audio_declared_method(method.channel(), method.effect()),
            Some(method),
            "every declared method resolves back from its channel and effect"
        );
    }
}

#[test]
fn a_request_for_another_channels_purpose_is_not_this_channels_relationship() {
    let speaker_fence = fence(0x11, 1);
    let microphone_request = request(AudioChannel::Microphone, "Endpoint/audio-microphone");
    assert_eq!(
        AdmittedAudioSession::new(
            AudioChannel::Speaker,
            microphone_request,
            speaker_fence,
            AudioSessionOrigin::Owner,
        ),
        Err(AudioMediatorError::EndpointBindingPurposeMismatch),
        "restating the microphone relationship as a speaker one is a different relationship"
    );
}

#[test]
fn an_attachment_an_audio_consumer_cannot_use_is_refused() {
    let speaker_fence = fence(0x11, 1);
    let listen = EndpointBindingRequest::new(
        ResourceRef::parse("Endpoint/audio-speaker").expect("endpoint"),
        ResourceRef::parse("Guest/workstation").expect("guest"),
        BindingSlot::parse("speaker").expect("slot"),
        EndpointAttachmentKind::Listen,
        BoundedToken::parse("audio-speaker-session").expect("purpose"),
    )
    .expect("endpoint binding request");
    assert_eq!(
        AdmittedAudioSession::new(
            AudioChannel::Speaker,
            listen,
            speaker_fence,
            AudioSessionOrigin::Owner,
        ),
        Err(AudioMediatorError::EndpointAttachmentUnsupported),
        "an audio consumer connects to the endpoint; it never owns one"
    );
}

#[test]
fn an_admitted_session_names_no_host_path_or_runtime_value() {
    let admitted = fence(0x11, 1);
    let speaker = session(
        AudioChannel::Speaker,
        "Endpoint/audio-speaker",
        &admitted,
        AudioSessionOrigin::Owner,
    );
    // Two derivations from the same committed spec are the same value, and
    // the value's whole content is the committed request plus its fence: it
    // has no accessor that could yield a socket path, a runtime directory, or
    // a tool, so no ambient runtime value has anything to redirect.
    let again = session(
        AudioChannel::Speaker,
        "Endpoint/audio-speaker",
        &admitted,
        AudioSessionOrigin::Owner,
    );
    assert_eq!(speaker, again);
    assert_eq!(speaker.source_ref().to_canonical_string(), "Endpoint/audio-speaker");
    assert_eq!(
        speaker.consumer_ref().to_canonical_string(),
        "Guest/workstation"
    );
    assert_eq!(speaker.request().slot().as_str(), "speaker");
    assert_eq!(
        speaker.request().purpose().as_str(),
        "audio-speaker-session"
    );
    assert_eq!(
        speaker.request().attachment(),
        EndpointAttachmentKind::Connect
    );
    assert_eq!(speaker.fence(), &admitted);
    assert_eq!(speaker.origin(), AudioSessionOrigin::Owner);

    // A different committed endpoint is a different relationship, never a
    // redirection of the admitted one.
    let other = session(
        AudioChannel::Speaker,
        "Endpoint/other-audio",
        &admitted,
        AudioSessionOrigin::Owner,
    );
    assert_ne!(speaker, other);
}

#[test]
fn speaker_and_microphone_effects_need_their_own_admitted_operations() {
    let speaker_fence = fence(0x11, 1);
    let plan = AudioSessionPlan::new(
        Some(session(
            AudioChannel::Speaker,
            "Endpoint/audio-speaker",
            &speaker_fence,
            AudioSessionOrigin::Owner,
        )),
        None,
    );
    let mut mediator = AdmittedAudioMediator::new(FakeAudioMediator::ready(), plan, vec![speaker_fence]);
    assert_eq!(
        mediator.set_channel_grant(AudioChannel::Speaker, AudioGrant::On),
        Ok(())
    );
    assert_eq!(
        mediator.set_channel_grant(AudioChannel::Microphone, AudioGrant::On),
        Err(AudioMediatorError::EndpointBindingNotAdmitted),
        "the speaker admission is not the microphone's admission"
    );
    assert_eq!(
        mediator.inner().channel_grant_calls(AudioChannel::Microphone),
        0,
        "the refused microphone effect never reached the session"
    );
}

#[test]
fn an_imported_projection_cannot_create_a_local_backing_grant() {
    let projected = fence(0x33, 1);
    let plan = AudioSessionPlan::new(
        Some(session(
            AudioChannel::Speaker,
            "Endpoint/audio-speaker",
            &projected,
            AudioSessionOrigin::ImportedProjection,
        )),
        None,
    );
    let mut mediator = AdmittedAudioMediator::new(
        FakeAudioMediator::ready(),
        plan,
        vec![projected.clone()],
    );
    assert_eq!(
        mediator.set_channel_grant(AudioChannel::Speaker, AudioGrant::On),
        Err(AudioMediatorError::ImportedProjectionCannotGrant),
        "an imported Service projection consumes the exporting Zone's session; it mints none locally"
    );
    assert_eq!(
        mediator.set_channel_level(AudioChannel::Speaker, LevelPercent::new(30).unwrap()),
        Err(AudioMediatorError::ImportedProjectionCannotGrant),
        "the level is a declared method on the same relationship and is refused with it"
    );
    assert_eq!(mediator.inner().grant(), AudioGrant::Off);
    assert_eq!(mediator.inner().grant_calls(), 0);
}

#[test]
fn revocation_keeps_the_host_and_guest_observations_separate() {
    let admitted = fence(0x11, 1);
    let speaker = session(
        AudioChannel::Speaker,
        "Endpoint/audio-speaker",
        &admitted,
        AudioSessionOrigin::Owner,
    );
    let plan = AudioSessionPlan::new(Some(speaker.clone()), None);
    let live =
        AdmittedAudioMediator::new(FakeAudioMediator::ready(), plan.clone(), vec![admitted.clone()]);
    assert_eq!(live.host_readiness(), HostAudioReadiness::Ready);
    assert_eq!(live.guest_readiness(), GuestAudioReadiness::Ready);
    assert_eq!(live.readiness(), AudioReadiness::Ready);

    // The relationship is re-committed and then revoked: the observed fence
    // no longer matches, so the host side stops serving effects while the
    // Guest's own agent observation is untouched.
    let revoked = AdmittedAudioMediator::new(
        FakeAudioMediator::ready(),
        plan,
        vec![fence(0x11, 2)],
    );
    assert_eq!(
        revoked.host_readiness(),
        HostAudioReadiness::Unavailable,
        "no current host relationship is no host readiness"
    );
    assert_eq!(
        revoked.guest_readiness(),
        GuestAudioReadiness::Ready,
        "the Guest observation is the Guest agent's, not the host session's"
    );
    let mut revoked = revoked;
    assert_eq!(
        revoked.set_channel_grant(AudioChannel::Speaker, AudioGrant::On),
        Err(AudioMediatorError::EndpointBindingNotCurrent)
    );
}

#[test]
fn a_restarted_controller_resumes_only_its_own_committed_relationship() {
    let admitted = fence(0x11, 1);
    let plan = AudioSessionPlan::new(
        Some(session(
            AudioChannel::Speaker,
            "Endpoint/audio-speaker",
            &admitted,
            AudioSessionOrigin::Owner,
        )),
        None,
    );
    let mut restarted =
        AdmittedAudioMediator::new(FakeAudioMediator::ready(), plan, vec![admitted]);
    assert_eq!(
        restarted.set_channel_grant(AudioChannel::Speaker, AudioGrant::On),
        Ok(()),
        "the same committed relationship admits the effect after a restart"
    );
    assert_eq!(restarted.inner().grant(), AudioGrant::On);
}

#[test]
fn an_owner_service_row_names_the_endpoint_and_operations_its_bindings_request() {
    use d2b_provider_audio_pipewire::AudioServiceSpec;

    let owner = AudioServiceSpec::owner(
        ResourceRef::parse("Endpoint/audio-host").expect("endpoint"),
        "work",
    );
    assert_eq!(
        service_backing_endpoint(&owner),
        Some(&ResourceRef::parse("Endpoint/audio-host").expect("endpoint")),
    );
    assert!(service_declares_channel(&owner, AudioChannel::Speaker));
    assert!(service_declares_channel(&owner, AudioChannel::Microphone));

    let mut playback_only = owner.clone();
    playback_only.operations = vec!["playback".to_owned()];
    assert!(service_declares_channel(&playback_only, AudioChannel::Speaker));
    assert!(
        !service_declares_channel(&playback_only, AudioChannel::Microphone),
        "declaring playback does not reach the capture methods"
    );

    let projection = AudioServiceSpec::projection("work").expect("projection row");
    assert_eq!(
        service_backing_endpoint(&projection),
        None,
        "an imported Service projection has no local backing endpoint to bind to"
    );
}

#[test]
fn a_channel_effect_resolves_to_its_own_declared_method() {
    assert_eq!(
        audio_declared_method(AudioChannel::Speaker, AudioEffectKind::Grant)
            .expect("speaker grant method")
            .name(),
        "set-speaker-grant"
    );
    assert_eq!(
        audio_declared_method(AudioChannel::Microphone, AudioEffectKind::Level)
            .expect("microphone gain method")
            .name(),
        "set-microphone-gain"
    );
}
