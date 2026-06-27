//! Integration tests for `d2b_core::audio_policy`.

use d2b_core::audio_policy::{
    AudioGrant, AudioPolicyError, AudioPolicyState, LevelPercent, LevelPercentError,
    parse_audio_state,
};
use d2b_core::provider_capabilities::{
    AudioGuestEnforcementKind, AudioHostEnforcementKind, AudioProviderCapability,
    ConsoleBackendKind, ConsoleProviderCapability,
};

// ── LevelPercent ─────────────────────────────────────────────────────────────

#[test]
fn level_percent_accepts_boundary_values() {
    assert_eq!(LevelPercent::new(0).unwrap().get(), 0);
    assert_eq!(LevelPercent::new(100).unwrap().get(), 100);
    assert_eq!(LevelPercent::new(50).unwrap().get(), 50);
}

#[test]
fn level_percent_rejects_over_100() {
    assert!(matches!(
        LevelPercent::new(101),
        Err(LevelPercentError::OutOfRange(101))
    ));
    assert!(matches!(
        LevelPercent::new(255),
        Err(LevelPercentError::OutOfRange(255))
    ));
}

#[test]
fn level_percent_round_trips_json() {
    let level = LevelPercent::new(73).unwrap();
    let json = serde_json::to_string(&level).unwrap();
    assert_eq!(json, "73");
    let back: LevelPercent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, level);
}

#[test]
fn level_percent_rejects_out_of_range_in_json() {
    assert!(serde_json::from_str::<LevelPercent>("101").is_err());
}

// ── AudioGrant ───────────────────────────────────────────────────────────────

#[test]
fn audio_grant_wire_strings() {
    assert_eq!(AudioGrant::On.as_wire_str(), "on");
    assert_eq!(AudioGrant::Off.as_wire_str(), "off");
    assert!(AudioGrant::On.is_on());
    assert!(!AudioGrant::Off.is_on());
}

#[test]
fn audio_grant_round_trips_json() {
    let on = serde_json::to_string(&AudioGrant::On).unwrap();
    let off = serde_json::to_string(&AudioGrant::Off).unwrap();
    assert_eq!(on, r#""on""#);
    assert_eq!(off, r#""off""#);
    assert_eq!(
        serde_json::from_str::<AudioGrant>(&on).unwrap(),
        AudioGrant::On
    );
    assert_eq!(
        serde_json::from_str::<AudioGrant>(&off).unwrap(),
        AudioGrant::Off
    );
}

// ── parse_audio_state – v1 ────────────────────────────────────────────────────

#[test]
fn parse_v1_both_on() {
    let state = parse_audio_state(br#"{"mic":"on","speaker":"on"}"#).unwrap();
    assert_eq!(state.mic, AudioGrant::On);
    assert_eq!(state.speaker, AudioGrant::On);
    assert_eq!(state.schema_version, "v2");
    assert!(state.speaker_level.is_none());
    assert!(state.mic_gain.is_none());
}

#[test]
fn parse_v1_mic_on_speaker_off() {
    let state = parse_audio_state(br#"{"mic":"on","speaker":"off"}"#).unwrap();
    assert_eq!(state.mic, AudioGrant::On);
    assert_eq!(state.speaker, AudioGrant::Off);
}

#[test]
fn parse_v1_both_off() {
    let state = parse_audio_state(br#"{"mic":"off","speaker":"off"}"#).unwrap();
    assert_eq!(state.mic, AudioGrant::Off);
    assert_eq!(state.speaker, AudioGrant::Off);
}

#[test]
fn parse_v1_unknown_grant_value_is_error() {
    let err = parse_audio_state(br#"{"mic":"maybe","speaker":"off"}"#).unwrap_err();
    assert!(
        matches!(&err, AudioPolicyError::InvalidField(msg) if msg.contains("mic")),
        "unexpected error: {err}"
    );
}

#[test]
fn parse_v1_missing_speaker_is_error() {
    let err = parse_audio_state(br#"{"mic":"on"}"#).unwrap_err();
    assert!(
        matches!(&err, AudioPolicyError::InvalidField(msg) if msg.contains("speaker")),
        "unexpected error: {err}"
    );
}

// ── parse_audio_state – v2 ────────────────────────────────────────────────────

#[test]
fn parse_v2_full_document() {
    let doc = br#"{
        "schemaVersion": "v2",
        "mic": "on",
        "speaker": "off",
        "speakerLevel": 75,
        "micGain": 80
    }"#;
    let state = parse_audio_state(doc).unwrap();
    assert_eq!(state.mic, AudioGrant::On);
    assert_eq!(state.speaker, AudioGrant::Off);
    assert_eq!(state.speaker_level.unwrap().get(), 75);
    assert_eq!(state.mic_gain.unwrap().get(), 80);
    assert_eq!(state.schema_version, "v2");
}

#[test]
fn parse_v2_omitted_levels_are_none() {
    let doc = br#"{"schemaVersion":"v2","mic":"off","speaker":"on"}"#;
    let state = parse_audio_state(doc).unwrap();
    assert!(state.speaker_level.is_none());
    assert!(state.mic_gain.is_none());
}

#[test]
fn parse_v2_explicit_null_levels_are_none() {
    // Explicit JSON null must be accepted as "unset; use system default".
    let doc = br#"{"schemaVersion":"v2","mic":"off","speaker":"on","speakerLevel":null,"micGain":null}"#;
    let state = parse_audio_state(doc).unwrap();
    assert!(state.speaker_level.is_none());
    assert!(state.mic_gain.is_none());
}

#[test]
fn parse_v2_level_out_of_range_is_error() {
    let doc =
        br#"{"schemaVersion":"v2","mic":"off","speaker":"off","speakerLevel":101}"#;
    assert!(matches!(
        parse_audio_state(doc),
        Err(AudioPolicyError::InvalidField(_))
    ));
}

#[test]
fn parse_unknown_schema_version_is_error() {
    let doc = br#"{"schemaVersion":"v99","mic":"off","speaker":"off"}"#;
    assert!(matches!(
        parse_audio_state(doc),
        Err(AudioPolicyError::UnknownSchemaVersion(v)) if v == "v99"
    ));
}

#[test]
fn parse_invalid_json_is_error() {
    assert!(matches!(
        parse_audio_state(b"not-json"),
        Err(AudioPolicyError::InvalidJson(_))
    ));
}

// ── to_v2_bytes / round-trip ──────────────────────────────────────────────────

#[test]
fn to_v2_bytes_produces_valid_json_with_schema_version() {
    let state = AudioPolicyState::default_v2()
        .with_mic(AudioGrant::On)
        .with_speaker(AudioGrant::Off)
        .with_speaker_level(LevelPercent::new(60).unwrap());
    let bytes = state.to_v2_bytes().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schemaVersion"], "v2");
    assert_eq!(value["mic"], "on");
    assert_eq!(value["speaker"], "off");
    assert_eq!(value["speakerLevel"], 60);
    assert!(value.get("micGain").map_or(true, |v| v.is_null()));
}

#[test]
fn v1_parse_then_v2_write_upgrades_format() {
    let v1 = br#"{"mic":"on","speaker":"off"}"#;
    let state = parse_audio_state(v1).unwrap();
    let bytes = state.to_v2_bytes().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schemaVersion"], "v2");
    assert_eq!(value["mic"], "on");
    assert_eq!(value["speaker"], "off");
}

#[test]
fn v2_round_trip_is_lossless() {
    let original = AudioPolicyState {
        schema_version: "v2".to_owned(),
        mic: AudioGrant::Off,
        speaker: AudioGrant::On,
        speaker_level: Some(LevelPercent::new(90).unwrap()),
        mic_gain: Some(LevelPercent::new(45).unwrap()),
    };
    let bytes = original.to_v2_bytes().unwrap();
    let parsed = parse_audio_state(&bytes).unwrap();
    assert_eq!(parsed.mic, original.mic);
    assert_eq!(parsed.speaker, original.speaker);
    assert_eq!(parsed.speaker_level, original.speaker_level);
    assert_eq!(parsed.mic_gain, original.mic_gain);
}

// ── Builder helpers ───────────────────────────────────────────────────────────

#[test]
fn builder_without_level_clears_field() {
    let state = AudioPolicyState::default_v2()
        .with_speaker_level(LevelPercent::new(50).unwrap())
        .without_speaker_level();
    assert!(state.speaker_level.is_none());
}

#[test]
fn builder_without_mic_gain_clears_field() {
    let state = AudioPolicyState::default_v2()
        .with_mic_gain(LevelPercent::new(30).unwrap())
        .without_mic_gain();
    assert!(state.mic_gain.is_none());
}

// ── Provider capability matrix ────────────────────────────────────────────────

#[test]
fn cloud_hypervisor_has_full_enforcement() {
    let cap = AudioProviderCapability::cloud_hypervisor_nixos();
    assert!(matches!(
        cap.host_enforcement,
        AudioHostEnforcementKind::PipeWireVhostUserSound
    ));
    assert!(matches!(
        cap.guest_enforcement,
        AudioGuestEnforcementKind::GuestdCapable
    ));
    assert!(cap.needs_local_state_file);
}

#[test]
fn qemu_media_has_host_only_enforcement() {
    let cap = AudioProviderCapability::qemu_media();
    assert!(matches!(
        cap.host_enforcement,
        AudioHostEnforcementKind::QemuAudioBackend
    ));
    assert!(matches!(
        cap.guest_enforcement,
        AudioGuestEnforcementKind::Unsupported
    ));
    assert!(cap.needs_local_state_file);
}

#[test]
fn aca_sandbox_has_no_host_enforcement() {
    let cap = AudioProviderCapability::aca_sandbox();
    assert!(matches!(
        cap.host_enforcement,
        AudioHostEnforcementKind::None
    ));
    assert!(matches!(
        cap.guest_enforcement,
        AudioGuestEnforcementKind::GuestdCapable
    ));
    assert!(!cap.needs_local_state_file);
}

#[test]
fn console_capabilities_cover_all_three_providers() {
    let ch = ConsoleProviderCapability::cloud_hypervisor_nixos();
    let qemu = ConsoleProviderCapability::qemu_media();
    let aca = ConsoleProviderCapability::aca_sandbox();
    assert!(matches!(ch.backend, ConsoleBackendKind::LocalHypervisor));
    assert!(ch.persistent_drain);
    assert!(matches!(qemu.backend, ConsoleBackendKind::LocalHypervisor));
    assert!(qemu.persistent_drain);
    assert!(matches!(aca.backend, ConsoleBackendKind::ProviderRelay));
    assert!(!aca.persistent_drain);
}

#[test]
fn provider_capabilities_round_trip_json() {
    let cap = AudioProviderCapability::cloud_hypervisor_nixos();
    let json = serde_json::to_string(&cap).unwrap();
    let back: AudioProviderCapability = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cap);

    let ccap = ConsoleProviderCapability::qemu_media();
    let json2 = serde_json::to_string(&ccap).unwrap();
    let back2: ConsoleProviderCapability = serde_json::from_str(&json2).unwrap();
    assert_eq!(back2, ccap);
}

#[test]
fn default_v2_state_is_all_off_no_levels() {
    let state = AudioPolicyState::default_v2();
    assert_eq!(state.mic, AudioGrant::Off);
    assert_eq!(state.speaker, AudioGrant::Off);
    assert!(state.speaker_level.is_none());
    assert!(state.mic_gain.is_none());
    assert_eq!(state.schema_version, "v2");
}
