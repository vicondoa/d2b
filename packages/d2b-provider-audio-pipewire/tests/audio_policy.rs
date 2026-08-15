use d2b_provider_audio_pipewire::{AudioGrant, AudioPolicyState, LevelPercent, parse_audio_state};

#[test]
fn legacy_audio_policy_migrates_to_v2_without_a_state_path() {
    let state = parse_audio_state(br#"{"mic":"on","speaker":"off"}"#).unwrap();
    assert_eq!(state.mic, AudioGrant::On);
    assert_eq!(state.speaker, AudioGrant::Off);
    assert_eq!(
        state.to_v2_bytes().unwrap(),
        br#"{"schemaVersion":"v2","mic":"on","speaker":"off"}"#
    );
}

#[test]
fn levels_are_bounded_and_grants_are_closed() {
    assert!(LevelPercent::new(100).is_ok());
    assert!(LevelPercent::new(101).is_err());
    assert!(serde_json::from_str::<AudioGrant>("\"maybe\"").is_err());
    assert_eq!(
        AudioPolicyState::default_v2()
            .with_speaker_level(LevelPercent::new(80).unwrap())
            .speaker_level
            .unwrap()
            .get(),
        80
    );
}
