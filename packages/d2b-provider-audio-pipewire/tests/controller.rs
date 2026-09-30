use d2b_contracts_resource::v3::{ExecutionDomain, ResourceRef};
use d2b_provider_audio_pipewire::{
    AudioArbitrationState, AudioBindingController, AudioBindingPhase, AudioChannel, AudioGrant,
    AudioLeaseId, AudioMediator, AudioMediatorError, AudioReadiness, AUDIO_QUEUE_BOUND,
    FakeAudioMediator, GuestAudioReadiness, HostAudioReadiness, LevelPercent,
    shared_microphone_arbiter, validate_audio_binding,
};

#[derive(Debug)]
struct HandoffFailureMediator {
    grant: AudioGrant,
    microphone_on_attempts: u8,
}

impl HandoffFailureMediator {
    fn new() -> Self {
        Self {
            grant: AudioGrant::Off,
            microphone_on_attempts: 0,
        }
    }

    fn set_microphone_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        if grant == AudioGrant::On {
            self.microphone_on_attempts += 1;
            if self.microphone_on_attempts == 2 {
                return Err(AudioMediatorError::ProviderSessionUnavailable);
            }
        }
        self.grant = grant;
        Ok(())
    }
}

impl AudioMediator for HandoffFailureMediator {
    fn set_grant(&mut self, grant: AudioGrant) -> Result<(), AudioMediatorError> {
        self.set_microphone_grant(grant)
    }

    fn set_channel_grant(
        &mut self,
        _channel: AudioChannel,
        grant: AudioGrant,
    ) -> Result<(), AudioMediatorError> {
        self.set_microphone_grant(grant)
    }

    fn set_level(&mut self, _level: LevelPercent) -> Result<(), AudioMediatorError> {
        Ok(())
    }

    fn readiness(&self) -> AudioReadiness {
        AudioReadiness::Ready
    }

    fn host_readiness(&self) -> HostAudioReadiness {
        HostAudioReadiness::Ready
    }

    fn guest_readiness(&self) -> GuestAudioReadiness {
        GuestAudioReadiness::Ready
    }
}

fn binding() -> d2b_provider_audio_pipewire::AudioBindingSpec {
    d2b_provider_audio_pipewire::AudioBindingSpec::new(
        ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").unwrap(),
        ResourceRef::parse("Guest/dev-vm").unwrap(),
        "zone-a",
    )
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
    assert_eq!(result.status.channels.speaker.grant, AudioGrant::On);
    assert!(result.status.channels.speaker.live_enforced);
    assert_eq!(
        result.status.channels.mic.arbitration_state,
        AudioArbitrationState::Inactive
    );
    assert_eq!(
        result.status.last_set_applied,
        d2b_provider_audio_pipewire::AudioLastSetApplied::HostAndGuest
    );
}

#[test]
fn controller_projects_levels_and_microphone_arbitration() {
    let mediator = FakeAudioMediator::ready();
    let mut controller = AudioBindingController::new(mediator);
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    requested.grants.speaker = AudioGrant::On;
    requested.grants.speaker_level = Some(LevelPercent::new(75).unwrap());
    requested.grants.mic_gain = Some(LevelPercent::new(40).unwrap());

    let result = controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();

    assert_eq!(
        result.status.channels.speaker.level,
        requested.grants.speaker_level
    );
    assert!(result.status.channels.speaker.live_enforced);
    assert_eq!(result.status.channels.mic.gain, requested.grants.mic_gain);
    assert!(result.status.channels.mic.live_enforced);
    assert_eq!(
        result.status.channels.mic.arbitration_state,
        AudioArbitrationState::Active
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
fn bindings_can_share_one_service_microphone_authority() {
    let shared = shared_microphone_arbiter(AUDIO_QUEUE_BOUND);
    let mut first =
        AudioBindingController::with_shared_microphone(FakeAudioMediator::ready(), shared.clone());
    let mut second =
        AudioBindingController::with_shared_microphone(FakeAudioMediator::ready(), shared);
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;

    assert_eq!(
        first
            .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
            .unwrap()
            .status
            .microphone,
        Some(d2b_provider_audio_pipewire::MicDecision::Granted)
    );
    assert_eq!(
        second
            .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
            .unwrap()
            .status
            .microphone,
        Some(d2b_provider_audio_pipewire::MicDecision::Queued)
    );
    assert_eq!(first.active_microphone_lease(), Some(AudioLeaseId::new(1)));
    assert_eq!(second.active_microphone_lease(), Some(AudioLeaseId::new(1)));
}

#[test]
fn shared_finalization_does_not_enable_the_promoted_binding_through_the_old_mediator() {
    let shared = shared_microphone_arbiter(AUDIO_QUEUE_BOUND);
    let mut first =
        AudioBindingController::with_shared_microphone(FakeAudioMediator::ready(), shared.clone());
    let mut second =
        AudioBindingController::with_shared_microphone(FakeAudioMediator::ready(), shared);
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    first
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    second
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();

    assert_eq!(
        first.finalize(AudioLeaseId::new(1)).unwrap(),
        Some(AudioLeaseId::new(2))
    );
    assert_eq!(first.mediator().microphone_grant(), AudioGrant::Off);
    assert_eq!(second.mediator().microphone_grant(), AudioGrant::Off);

    second
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();
    assert_eq!(second.mediator().microphone_grant(), AudioGrant::On);
}

#[test]
fn failed_microphone_promotion_requeues_the_promoted_lease() {
    let mut controller = AudioBindingController::new(HandoffFailureMediator::new());
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();

    assert_eq!(
        controller.finalize(AudioLeaseId::new(1)).unwrap_err(),
        d2b_provider_audio_pipewire::AudioControllerError::Mediator(
            AudioMediatorError::ProviderSessionUnavailable
        )
    );
    assert_eq!(controller.active_microphone_lease(), None);

    let result = controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();
    assert_eq!(
        result.status.microphone,
        Some(d2b_provider_audio_pipewire::MicDecision::Granted)
    );
    assert_eq!(
        controller.active_microphone_lease(),
        Some(AudioLeaseId::new(2))
    );
}

#[test]
fn unchanged_audio_reconciliation_does_not_repeat_mediator_effects() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    requested.grants.speaker = AudioGrant::On;
    requested.grants.speaker_level = Some(LevelPercent::new(25).expect("bounded test level"));

    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    let first_counts = (
        controller.mediator().grant_calls(),
        controller.mediator().level_calls(),
    );
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    assert_eq!(
        (
            controller.mediator().grant_calls(),
            controller.mediator().level_calls()
        ),
        first_counts
    );
}

#[test]
fn speaker_admission_rejects_before_mutating_mediator() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.speaker_level =
        Some(d2b_provider_audio_pipewire::LevelPercent::new(25).expect("bounded test level"));
    for lease in 1..=AUDIO_QUEUE_BOUND.get() as u64 {
        controller
            .reconcile(&requested, "zone-a", AudioLeaseId::new(lease))
            .unwrap();
    }
    let last_level = controller.mediator().level();
    assert_eq!(
        controller
            .reconcile(&requested, "zone-a", AudioLeaseId::new(AUDIO_QUEUE_BOUND.get() as u64 + 1))
            .unwrap_err(),
        d2b_provider_audio_pipewire::AudioControllerError::Admission
    );
    assert_eq!(controller.mediator().level(), last_level);
}

#[test]
fn failed_microphone_effect_rolls_back_the_arbitration_lease() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::unavailable());
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;

    assert_eq!(
        controller
            .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
            .unwrap_err(),
        d2b_provider_audio_pipewire::AudioControllerError::Mediator(
            AudioMediatorError::ProviderSessionUnavailable
        )
    );
    assert_eq!(controller.active_microphone_lease(), None);
}

#[test]
fn queued_microphone_reconcile_does_not_mute_the_active_owner() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();

    requested.grants.mic = AudioGrant::Off;
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();

    assert_eq!(
        controller.active_microphone_lease(),
        Some(AudioLeaseId::new(1))
    );
    assert_eq!(controller.mediator().microphone_grant(), AudioGrant::On);
}

#[test]
fn finalization_mutes_before_promoting_the_next_microphone_owner() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.mic = AudioGrant::On;
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();

    assert_eq!(
        controller.finalize(AudioLeaseId::new(1)).unwrap(),
        Some(AudioLeaseId::new(2))
    );
    assert_eq!(
        controller.active_microphone_lease(),
        Some(AudioLeaseId::new(2))
    );
    assert_eq!(controller.mediator().microphone_grant(), AudioGrant::On);
}

#[test]
fn speaker_release_keeps_other_consumers_granted() {
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
    let mut requested = binding();
    requested.grants.speaker = AudioGrant::On;
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(2))
        .unwrap();

    requested.grants.speaker = AudioGrant::Off;
    controller
        .reconcile(&requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();

    assert_eq!(controller.mediator().grant(), AudioGrant::On);
}

#[test]
fn explicit_binding_reconciliation_returns_host_and_guest_children() {
    let binding_ref =
        ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").expect("canonical Binding");
    let requested = binding();
    let mut controller = AudioBindingController::new(FakeAudioMediator::ready());

    let result = controller
        .reconcile_with_children(&binding_ref, &requested, "zone-a", AudioLeaseId::new(1))
        .unwrap();

    assert_eq!(result.children.iter().count(), 4);
    assert_eq!(
        result
            .children
            .at(
                d2b_contracts_provider::v3::semantic_services::child_resources::
                    BindingChildPlacement::Host,
            )
            .count(),
        2
    );
    assert_eq!(
        result
            .children
            .at(
                d2b_contracts_provider::v3::semantic_services::child_resources::
                    BindingChildPlacement::Guest,
            )
            .count(),
        2
    );
    assert_eq!(
        result.children.teardown_order().first().unwrap().kind(),
        d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildKind::Endpoint
    );
    let guest_process = result.children.child("guest-agent").unwrap();
    assert_eq!(guest_process.execution_ref(), &requested.target_ref);
    assert_eq!(
        guest_process.process_provider(),
        Some("Provider/system-systemd")
    );
    assert_eq!(guest_process.process_template(), Some("guest-audio-agent"));
    assert_eq!(guest_process.process_class(), Some("service"));
    assert_eq!(
        guest_process.process_domain(),
        Some(ExecutionDomain::System)
    );
    assert!(guest_process.process_user().is_none());
}

#[test]
fn ready_audio_service_without_an_authored_binding_has_no_children() {
    let service_ref =
        ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").expect("canonical Service");
    let target_ref = ResourceRef::parse("Guest/dev-vm").expect("canonical Guest");
    let service_only_ref = service_ref.clone();
    let service_only =
        d2b_provider_audio_pipewire::AudioBindingSpec::new(service_ref, target_ref, "zone-a");

    assert_eq!(
        AudioBindingController::<FakeAudioMediator>::child_resources(
            &service_only_ref,
            &service_only,
        )
        .unwrap_err(),
        d2b_provider_audio_pipewire::AudioControllerError::Admission
    );
}

// ── admitted endpoint relationships ──────────────────────────────────────────

mod admitted {
    use d2b_contracts_resource::v3::{
        BindingSlot, EndpointAttachmentKind, EndpointBindingRequest, ResourceGeneration,
        ResourceRef, ResourceUid, ZoneRevision, execution_policy::BoundedToken,
    };
    use d2b_provider_audio_pipewire::{
        AdmittedAudioSession, AudioBindingController, AudioBindingFence, AudioBindingSpec,
        AudioChannel, AudioGrant, AudioMediatorError, AudioSessionOrigin,
        AudioSessionPlan, FakeAudioMediator, LevelPercent, validate_audio_binding,
        validate_audio_binding_in_zone, validate_audio_service, AudioServiceSpec,
    };

    fn fence(byte: u8, generation: u64) -> AudioBindingFence {
        AudioBindingFence::new(
            ResourceUid::from_bytes(&[byte; 16]).expect("uuid"),
            ResourceGeneration::new(generation).expect("generation"),
            ZoneRevision::new(7),
        )
    }

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

    fn session(
        channel: AudioChannel,
        fence: &AudioBindingFence,
        origin: AudioSessionOrigin,
    ) -> AdmittedAudioSession {
        AdmittedAudioSession::new(
            channel,
            request(channel, "Endpoint/audio-host"),
            fence.clone(),
            origin,
        )
        .expect("admitted session")
    }

    fn binding() -> AudioBindingSpec {
        AudioBindingSpec::new(
            ResourceRef::parse("audio.d2bus.org.AudioService/host-audio").expect("service"),
            ResourceRef::parse("Guest/workstation").expect("target"),
            "zone-a",
        )
    }

    #[test]
    fn the_owner_service_declares_the_endpoint_its_bindings_request() {
        let spec = AudioServiceSpec::owner(
            ResourceRef::parse("Endpoint/audio-host").expect("endpoint"),
            "zone-a",
        );
        validate_audio_service(&spec).expect("the owner row validates");
        assert_eq!(spec.implementation_endpoint_refs.len(), 1);
        assert!(spec.operations.contains(&"playback".to_owned()));
        assert!(spec.operations.contains(&"capture".to_owned()));
    }

    #[test]
    fn an_imported_projection_declares_no_local_backing_endpoint() {
        let spec = AudioServiceSpec::projection("zone-a").expect("projection row");
        validate_audio_service(&spec).expect("the projection row validates");
        assert!(
            spec.implementation_endpoint_refs.is_empty(),
            "an imported Service has no local endpoint a binding could request"
        );
    }

    #[test]
    fn the_admitted_pass_carries_each_channel_through_its_own_relationship() {
        let speaker_fence = fence(0x11, 1);
        let microphone_fence = fence(0x22, 1);
        let plan = AudioSessionPlan::new(
            Some(session(
                AudioChannel::Speaker,
                &speaker_fence,
                AudioSessionOrigin::Owner,
            )),
            Some(session(
                AudioChannel::Microphone,
                &microphone_fence,
                AudioSessionOrigin::Owner,
            )),
        );
        let mut requested = binding();
        requested.grants.speaker = AudioGrant::On;
        requested.grants.mic = AudioGrant::On;
        requested.grants.speaker_level = Some(LevelPercent::new(35).expect("level"));

        let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
        controller
            .reconcile_admitted(
                &requested,
                "zone-a",
                d2b_provider_audio_pipewire::AudioLeaseId::new(1),
                &plan,
                &[speaker_fence.clone(), microphone_fence.clone()],
            )
            .expect("the admitted pass reconciles");
        assert_eq!(controller.mediator().grant(), AudioGrant::On);
        assert_eq!(controller.mediator().microphone_grant(), AudioGrant::On);
        assert_eq!(
            controller.mediator().level(),
            Some(LevelPercent::new(35).expect("level"))
        );
    }

    #[test]
    fn an_unadmitted_channel_is_refused_before_the_effect_runs() {
        let speaker_fence = fence(0x11, 1);
        let plan = AudioSessionPlan::new(
            Some(session(
                AudioChannel::Speaker,
                &speaker_fence,
                AudioSessionOrigin::Owner,
            )),
            None,
        );
        let mut requested = binding();
        requested.grants.speaker = AudioGrant::On;
        requested.grants.mic = AudioGrant::On;

        let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
        let failure = controller
            .reconcile_admitted(
                &requested,
                "zone-a",
                d2b_provider_audio_pipewire::AudioLeaseId::new(1),
                &plan,
                &[speaker_fence],
            )
            .expect_err("the microphone has no admitted relationship");
        assert_eq!(
            failure,
            d2b_provider_audio_pipewire::AudioControllerError::Mediator(
                AudioMediatorError::EndpointBindingNotAdmitted
            ),
            "the refusal names the absent relationship, not a generic failure"
        );
        assert_eq!(
            controller.mediator().grant(),
            AudioGrant::Off,
            "the pass refuses before any channel effect runs, so the speaker \
             grant is not applied alongside a refused microphone"
        );
        assert_eq!(
            controller.mediator().grant_calls(),
            0,
            "no channel effect reached the session"
        );
    }

    #[test]
    fn an_imported_projection_cannot_reach_a_local_host_grant() {
        let projected = fence(0x33, 1);
        let plan = AudioSessionPlan::new(
            Some(session(
                AudioChannel::Speaker,
                &projected,
                AudioSessionOrigin::ImportedProjection,
            )),
            None,
        );
        let mut requested = binding();
        requested.grants.speaker = AudioGrant::On;

        let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
        let failure = controller
            .reconcile_admitted(
                &requested,
                "zone-a",
                d2b_provider_audio_pipewire::AudioLeaseId::new(1),
                &plan,
                &[projected],
            )
            .expect_err("a projection cannot grant locally");
        assert_eq!(
            failure,
            d2b_provider_audio_pipewire::AudioControllerError::Mediator(
                AudioMediatorError::ImportedProjectionCannotGrant
            )
        );
        assert_eq!(controller.mediator().grant(), AudioGrant::Off);
    }

    #[test]
    fn restart_reports_host_and_guest_observations_separately() {
        let admitted = fence(0x11, 1);
        let plan = AudioSessionPlan::new(
            Some(session(
                AudioChannel::Speaker,
                &admitted,
                AudioSessionOrigin::Owner,
            )),
            None,
        );
        let mut requested = binding();
        requested.grants.speaker = AudioGrant::On;

        let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
        let live = controller
            .reconcile_admitted(
                &requested,
                "zone-a",
                d2b_provider_audio_pipewire::AudioLeaseId::new(1),
                &plan,
                &[admitted.clone()],
            )
            .expect("the admitted pass reconciles");
        assert_eq!(
            live.status.host_readiness,
            d2b_provider_audio_pipewire::HostAudioReadiness::Ready
        );
        assert_eq!(
            live.status.guest_readiness,
            d2b_provider_audio_pipewire::GuestAudioReadiness::Ready
        );

        // After the relationship is re-committed, a restarted controller
        // observes the host side as unavailable while the Guest's own
        // observation is unchanged.
        let mut restarted = AudioBindingController::new(FakeAudioMediator::ready());
        let failure = restarted
            .reconcile_admitted(
                &requested,
                "zone-a",
                d2b_provider_audio_pipewire::AudioLeaseId::new(1),
                &plan,
                &[fence(0x11, 2)],
            )
            .expect_err("the re-committed relationship is not the admitted one");
        assert_eq!(
            failure,
            d2b_provider_audio_pipewire::AudioControllerError::Mediator(
                AudioMediatorError::EndpointBindingNotCurrent
            )
        );
    }

    #[test]
    fn a_cross_zone_binding_is_refused_before_any_relationship_is_requested() {
        let mut cross_zone = binding();
        cross_zone.zone = "zone-b".to_owned();
        let speaker_fence = fence(0x11, 1);
        let plan = AudioSessionPlan::new(
            Some(session(
                AudioChannel::Speaker,
                &speaker_fence,
                AudioSessionOrigin::Owner,
            )),
            None,
        );
        let mut controller = AudioBindingController::new(FakeAudioMediator::ready());
        let failure = controller
            .reconcile_admitted(
                &cross_zone,
                "zone-a",
                d2b_provider_audio_pipewire::AudioLeaseId::new(1),
                &plan,
                &[speaker_fence],
            )
            .expect_err("a binding cannot realize from another Zone's Service");
        assert_eq!(
            failure,
            d2b_provider_audio_pipewire::AudioControllerError::Admission
        );
        validate_audio_binding(&cross_zone).expect("the row itself is well formed");
        assert_eq!(
            validate_audio_binding_in_zone(&cross_zone, "zone-a"),
            Err(d2b_provider_audio_pipewire::AudioAdmissionError::CrossZone)
        );
    }
}
