    use super::*;
    use std::collections::BTreeSet;
    use d2b_contracts_resource::v3::ZoneRevision;
    use d2b_contracts_resource::v3::CanonicalJsonValue;

    fn stored_audio_resource(resource_ref: &str, spec: serde_json::Value) -> StoredResource {
        let resource_ref = ResourceRef::parse(resource_ref).unwrap();
        let zone = ZoneId::parse("dev").unwrap();
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_ref.resource_type().as_str(),
            "metadata": {
                "name": resource_ref.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": null,
                "labels": {},
                "annotations": {},
                "finalizers": [],
                "managedBy": "controller",
                "configurationGeneration": 1,
                "deletionRequestedAt": null,
                "createdAt": "2026-08-19T00:00:00.000Z",
                "updatedAt": "2026-08-19T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "uid": "123e4567-e89b-42d3-a456-426614174000"
            },
            "spec": spec,
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "lastReconciledAt": null,
                "startedAt": null,
                "completedAt": null,
                "outcome": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                },
                "resource": {}
            }
        });
        let canonical = CanonicalJsonValue::parse(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .to_canonical_bytes();
        StoredResource {
            resource_ref,
            zone,
            uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .unwrap(),
            owner_uid: None,
            owner_generation: None,
            generation: d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            canonical_json: canonical,
            payload_digest: "sha256:test".to_owned(),
        }
    }

    #[cfg(test)]
    fn validate_relationships(
        services: &BTreeMap<String, AudioServiceSpec>,
        bindings: &[(String, (StoredResource, AudioBindingSpec))],
        guests: &BTreeSet<String>,
    ) -> Result<(), AudioResourceRuntimeError> {
        for (_, (resource, spec)) in bindings {
            if (!deletion_requested(resource)
                && (!services.contains_key(&spec.service_ref.to_canonical_string())
                    || !guests.contains(&spec.target_ref.to_canonical_string())))
                || resource.resource_ref.resource_type().as_str() != AUDIO_BINDING_TYPE
            {
                return Err(AudioResourceRuntimeError::InvalidRelationship);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn decode_services(
        zone: &ZoneId,
        resources: &[StoredResource],
    ) -> Result<BTreeMap<String, AudioServiceSpec>, AudioResourceRuntimeError> {
        let mut services = BTreeMap::new();
        for resource in resources {
            if !is_audio_resource(resource, zone)? {
                continue;
            }
            let spec: AudioServiceSpec = decode_spec(resource)?;
            if validate_audio_service(&spec).is_err() {
                return Err(AudioResourceRuntimeError::InvalidResource);
            }
            let key = resource.resource_ref.to_canonical_string();
            if services.insert(key, spec).is_some() {
                return Err(AudioResourceRuntimeError::InvalidResource);
            }
        }
        Ok(services)
    }

    #[test]
    fn audio_lease_identity_is_stable_and_nonzero() {
        let resource = ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap();
        assert_eq!(lease_for(&resource), lease_for(&resource));
        assert_ne!(
            lease_for(&resource),
            lease_for(&ResourceRef::parse("audio.d2bus.org.AudioBinding/other").unwrap())
        );
    }

    #[test]
    fn audio_status_projection_is_stable_and_separates_readiness() {
        let status = audio_binding_status_value(unavailable_status(
            AudioBindingPhase::Degraded,
            HostAudioReadiness::Ready,
            GuestAudioReadiness::Unavailable,
        ));
        assert_eq!(status["phase"], "Degraded");
        assert_eq!(status["hostReadiness"], "Ready");
        assert_eq!(status["guestReadiness"], "Unavailable");
        assert!(status["microphone"].is_null());
        assert_eq!(status["channels"]["speaker"]["grant"], "off");
        assert_eq!(status["channels"]["mic"]["grant"], "off");
        assert_eq!(status["channels"]["mic"]["arbitrationState"], "inactive");
        assert_eq!(status["enforcementPosture"], "None");
        assert_eq!(status["lastSetApplied"], "OfflineOnly");
    }

    #[test]
    fn audio_resource_projection_matches_the_frozen_status_schema() {
        let binding = AudioBindingSpec::new(
            ResourceRef::parse("audio.d2bus.org.AudioService/owner").unwrap(),
            ResourceRef::parse("Guest/work").unwrap(),
            "dev",
        )
        .unwrap();
        let mut status = unavailable_status(
            AudioBindingPhase::Degraded,
            HostAudioReadiness::Unavailable,
            GuestAudioReadiness::Unavailable,
        );
        status.channels.speaker.grant = binding.grants.speaker;
        status.channels.speaker.level = binding.grants.speaker_level;
        status.channels.mic.grant = binding.grants.mic;
        status.channels.mic.gain = binding.grants.mic_gain;
        let projection = audio_binding_projection(&binding, &[], &status);
        let names = projection
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        d2b_contracts_provider::v3::semantic_services::SemanticFamily::Audio
            .contract()
            .binding()
            .status()
            .validate_names(names)
            .expect("audio projection matches the frozen status schema");
        assert_eq!(
            projection["observedServiceRef"],
            "audio.d2bus.org.AudioService/owner"
        );
        assert_eq!(projection["realizationRefs"], serde_json::json!([]));
        assert_eq!(projection["channels"]["speaker"]["grant"], "off");
        assert_eq!(projection["channels"]["mic"]["grant"], "off");
        assert_eq!(projection["enforcementPosture"], "None");
        assert_eq!(projection["lastSetApplied"], "OfflineOnly");
    }

    #[test]
    fn relationship_validation_rejects_missing_guest_and_cross_service() {
        let zone = ZoneId::parse("dev").unwrap();
        let service_ref = ResourceRef::parse("audio.d2bus.org.AudioService/owner").unwrap();
        let guest_ref = ResourceRef::parse("Guest/vm").unwrap();
        let binding_ref = ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap();
        let service =
            AudioServiceSpec::owner(ResourceRef::parse("Endpoint/audio").unwrap(), zone.as_str())
                .unwrap();
        let binding =
            AudioBindingSpec::new(service_ref.clone(), guest_ref.clone(), zone.as_str()).unwrap();
        let resource = StoredResource {
            resource_ref: binding_ref.clone(),
            zone: zone.clone(),
            uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .unwrap(),
            owner_uid: None,
            owner_generation: None,
            generation: d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            revision: ZoneRevision::new(1),
            canonical_json: br#"{"metadata":{}}"#.to_vec(),
            payload_digest: String::new(),
        };
        let bindings = vec![(
            resource.resource_ref.to_canonical_string(),
            (resource, binding),
        )];
        let mut services = BTreeMap::new();
        services.insert(service_ref.to_canonical_string(), service);
        assert_eq!(
            validate_relationships(&services, &bindings, &BTreeSet::new()),
            Err(AudioResourceRuntimeError::InvalidRelationship)
        );
        assert_eq!(
            validate_relationships(
                &services,
                &bindings,
                &BTreeSet::from([guest_ref.to_canonical_string()])
            ),
            Ok(())
        );
        let mut deleting_resource = bindings[0].1.0.clone();
        deleting_resource.canonical_json =
            br#"{"metadata":{"deletionRequestedAt":"2026-08-15T00:00:00Z"}}"#.to_vec();
        let deleting_bindings = vec![(
            binding_ref.to_canonical_string(),
            (deleting_resource, bindings[0].1.1.clone()),
        )];
        assert_eq!(
            validate_relationships(&BTreeMap::new(), &deleting_bindings, &BTreeSet::new()),
            Ok(())
        );
    }

    #[test]
    fn audio_decoder_reads_reserved_provider_ref_from_resource_spec() {
        let zone = ZoneId::parse("dev").unwrap();
        let spec =
            AudioServiceSpec::owner(ResourceRef::parse("Endpoint/audio").unwrap(), zone.as_str())
                .unwrap();
        let resource = stored_audio_resource(
            "audio.d2bus.org.AudioService/owner",
            serde_json::to_value(spec).unwrap(),
        );

        let services = decode_services(&zone, &[resource]).unwrap();
        assert_eq!(
            services
                .get("audio.d2bus.org.AudioService/owner")
                .unwrap()
                .provider_ref,
            PROVIDER_REF
        );
    }

    #[test]
    fn audio_decoder_ignores_a_foreign_provider_resource() {
        let zone = ZoneId::parse("dev").unwrap();
        let resource = stored_audio_resource(
            "audio.d2bus.org.AudioService/foreign",
            serde_json::json!({
                "providerRef": "Provider/other",
                "implementationDetail": true
            }),
        );

        assert!(decode_services(&zone, &[resource]).unwrap().is_empty());
    }
struct NoAudioSource;

    impl AudioMediatorSource for NoAudioSource {

        fn build(&self, _vm_name: &str, _projection: bool) -> Option<Box<dyn AudioMediator>> {
            None
        }
    }

    struct FailingMediatorSource;

    impl AudioMediatorSource for FailingMediatorSource {

        fn build(&self, _vm_name: &str, _projection: bool) -> Option<Box<dyn AudioMediator>> {
            Some(Box::new(d2b_provider_audio_pipewire::FakeAudioMediator::unavailable()))
        }
    }

    fn service_resource(zone: &ZoneId) -> StoredResource {
        stored_audio_resource(
            "audio.d2bus.org.AudioService/owner",
            serde_json::to_value(
                AudioServiceSpec::owner(ResourceRef::parse("Endpoint/audio").unwrap(), zone.as_str())
                    .unwrap(),
            )
            .unwrap(),
        )
    }

    fn binding_resource(zone: &ZoneId, mic_on: bool) -> StoredResource {
        let mut spec = AudioBindingSpec::new(
            ResourceRef::parse("audio.d2bus.org.AudioService/owner").unwrap(),
            ResourceRef::parse("Guest/vm").unwrap(),
            zone.as_str(),
        )
        .unwrap();
        if mic_on {
            spec.grants.mic = AudioGrant::On;
        }
        stored_audio_resource(
            "audio.d2bus.org.AudioBinding/mic",
            serde_json::to_value(spec).unwrap(),
        )
    }

    fn guest_resource(name: &str) -> StoredResource {
        stored_audio_resource(name, serde_json::json!({}))
    }

    #[test]
    fn reconcile_binding_resource_validates_relationships_and_reconciles() {
        let zone = ZoneId::parse("dev").unwrap();

        // A service that is not the binding's declared same-Zone service.



        let mut runtime = AudioResourceRuntime::new(zone.clone(), Arc::new(NoAudioSource));
        let other_service = stored_audio_resource(
            "audio.d2bus.org.AudioService/other",
            serde_json::to_value(
                AudioServiceSpec::owner(ResourceRef::parse("Endpoint/audio").unwrap(), zone.as_str())
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            runtime.reconcile_binding_resource(
                &binding_resource(&zone, false),
                &other_service,
                &guest_resource("Guest/vm"),
            ),
            Err(AudioResourceRuntimeError::InvalidRelationship)
        );

        // A Guest row that is not the binding's target refuses, as does a
        // Guest row whose envelope does not decode.

                let mut broken_guest = guest_resource("Guest/vm");
        broken_guest.canonical_json = b"{}".to_vec();
        let mut runtime = AudioResourceRuntime::new(zone.clone(), Arc::new(NoAudioSource));
        assert_eq!(
            runtime.reconcile_binding_resource(
                &binding_resource(&zone, false),
                &service_resource(&zone),
                &broken_guest,
            ),
            Err(AudioResourceRuntimeError::InvalidRelationship)
        );

        // A valid trio reconciles: no audio capability on the target
        // publishes the honest Degraded/Unavailable status and the binding row
        // enters the shared registry.



        let mut runtime = AudioResourceRuntime::new(zone.clone(), Arc::new(NoAudioSource));
        let status = runtime
            .reconcile_binding_resource(
                &binding_resource(&zone, false),
                &service_resource(&zone),
                &guest_resource("Guest/vm"),
            )
            .unwrap()
            .expect("binding status");
        assert_eq!(status.status.phase, AudioBindingPhase::Degraded);
        assert_eq!(status.status.host_readiness, HostAudioReadiness::Unavailable);
        assert_eq!(status.status.guest_readiness, GuestAudioReadiness::Unavailable);
        assert_eq!(runtime.statuses().len(), 1);
    }

    #[test]
    fn controller_error_maps_to_a_degraded_binding_status() {
        let zone = ZoneId::parse("dev").unwrap();
        let mut runtime = AudioResourceRuntime::new(zone.clone(), Arc::new(FailingMediatorSource));
        let status = runtime
            .reconcile_binding_resource(
                &binding_resource(&zone, true),
                &service_resource(&zone),
                &guest_resource("Guest/vm"),
            )
            .unwrap()
            .expect("binding status");
        assert_eq!(status.status.phase, AudioBindingPhase::Degraded);
        assert_eq!(
            status.status.host_readiness,
            HostAudioReadiness::Unavailable,
            "the failing mediator's host readiness is surfaced"
        );
        assert_eq!(status.status.guest_readiness, GuestAudioReadiness::Ready);
    }

    #[test]
    fn product_is_audio_resource_and_decode_spec_are_pinned_directly() {
        let zone = ZoneId::parse("dev").unwrap();

        // A non-audio type,and an unreadable envelope are unusable identities. other provider rows are not audio resources.


        assert_eq!(
            is_audio_resource(
                &stored_audio_resource("Process/audio-service", serde_json::json!({})),
                &zone,
            ),
            Err(AudioResourceRuntimeError::InvalidResource)
        );
        let mut broken = service_resource(&zone);
        broken.canonical_json = b"{}".to_vec();
        assert_eq!(
            is_audio_resource(&broken, &zone),
            Err(AudioResourceRuntimeError::InvalidResource)
        );
        let foreign_provider = stored_audio_resource(
            "audio.d2bus.org.AudioService/foreign",
            serde_json::json!({
                "providerRef": "Provider/other",
                "implementationDetail": true
            }),
        );
        assert_eq!(is_audio_resource(&foreign_provider, &zone), Ok(false));
        assert_eq!(is_audio_resource(&service_resource(&zone), &zone), Ok(true));

        // The spec decoder re-inserts the reserved providerRef and round-trips
        // the typed spec.


        let decoded: AudioServiceSpec = decode_spec(&service_resource(&zone)).unwrap();
        assert_eq!(decoded.provider_ref, PROVIDER_REF);
        let decoded_binding: AudioBindingSpec = decode_spec(&binding_resource(&zone, false)).unwrap();
        assert_eq!(decoded_binding.provider_ref, PROVIDER_REF);
        assert_eq!(
            decoded_binding.service_ref,
            ResourceRef::parse("audio.d2bus.org.AudioService/owner").unwrap()
        );
        assert_eq!(
            decode_spec::<AudioServiceSpec>(&broken),
            Err(AudioResourceRuntimeError::InvalidResource)
        );
    }



