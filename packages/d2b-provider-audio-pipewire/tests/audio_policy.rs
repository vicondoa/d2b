//! Integration tests for `d2b_provider_audio_pipewire`.

use d2b_provider_audio_pipewire::{
    AudioGrant, AudioPolicyError, AudioPolicyState, LevelPercent, LevelPercentError,
    parse_audio_state,
};

// ── Wire shapes: LevelPercent / AudioGrant ──────────────────────────────────

#[test]
fn level_and_grant_wire_shapes_are_pinned() {
    // AudioGrant wire spellings and JSON round trips.
    for (name, grant, wire) in [("on", AudioGrant::On, "on"), ("off", AudioGrant::Off, "off")] {
        assert_eq!(grant.as_wire_str(), wire, "row: {name}");
        assert_eq!(
            serde_json::to_string(&grant).unwrap(),
            format!(r#""{wire}""#),
            "row: {name}"
        );
        assert_eq!(
            serde_json::from_str::<AudioGrant>(&format!(r#""{wire}""#)).unwrap(),
            grant,
            "row: {name}"
        );
    }
    assert!(AudioGrant::On.is_on());
    assert!(!AudioGrant::Off.is_on());

    // LevelPercent accepts the closed 0..=100 range.
    for (name, raw) in [("zero", 0_u8), ("mid", 50), ("hundred", 100)] {
        assert_eq!(LevelPercent::new(raw).unwrap().get(), raw, "row: {name}");
    }
    // ...and refuses everything above it, on the wire and in JSON.
    for (name, raw) in [("one over", 101_u8), ("max", 255)] {
        assert!(
            matches!(LevelPercent::new(raw), Err(LevelPercentError::OutOfRange(v)) if v == raw),
            "row: {name}"
        );
        assert!(serde_json::from_str::<LevelPercent>(&raw.to_string()).is_err(), "row: {name}");
    }
}

// ── parse_audio_state - v1 ────────────────────────────────────────────────────

#[test]
fn parse_v1_wire_shapes() {
    // Success rows: a v1 document parses with the v2 schema version and no
    // levels attached.
    for (name, doc, mic, speaker) in [
        ("both on", r#"{"mic":"on","speaker":"on"}"#, AudioGrant::On, AudioGrant::On),
        (
            "mic on speaker off",
            r#"{"mic":"on","speaker":"off"}"#,
            AudioGrant::On,
            AudioGrant::Off,
        ),
        (
            "both off",
            r#"{"mic":"off","speaker":"off"}"#,
            AudioGrant::Off,
            AudioGrant::Off,
        ),
    ] {
        let state = parse_audio_state(doc.as_bytes()).unwrap();
        assert_eq!(state.mic, mic, "row: {name}");
        assert_eq!(state.speaker, speaker, "row: {name}");
        assert_eq!(state.schema_version, "v2", "row: {name}");
        assert!(state.speaker_level.is_none(), "row: {name}");
        assert!(state.mic_gain.is_none(), "row: {name}");
    }

    // Error rows: a malformed grant field is refused with the field named.
    for (name, doc, field) in [
        ("unknown grant value", r#"{"mic":"maybe","speaker":"off"}"#, "mic"),
        ("missing speaker", r#"{"mic":"on"}"#, "speaker"),
    ] {
        let err = parse_audio_state(doc.as_bytes()).unwrap_err();
        assert!(
            matches!(&err, AudioPolicyError::InvalidField(msg) if msg.contains(field)),
            "row: {name}, unexpected error: {err}"
        );
    }
}

// ── parse_audio_state - v2 ────────────────────────────────────────────────────

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
    let doc =
        br#"{"schemaVersion":"v2","mic":"off","speaker":"on","speakerLevel":null,"micGain":null}"#;
    let state = parse_audio_state(doc).unwrap();
    assert!(state.speaker_level.is_none());
    assert!(state.mic_gain.is_none());
}

#[test]
fn parse_v2_level_out_of_range_is_error() {
    let doc = br#"{"schemaVersion":"v2","mic":"off","speaker":"off","speakerLevel":101}"#;
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
    assert!(value.get("micGain").is_none_or(|v| v.is_null()));
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

#[test]
fn default_v2_state_is_all_off_no_levels() {
    let state = AudioPolicyState::default_v2();
    assert_eq!(state.mic, AudioGrant::Off);
    assert_eq!(state.speaker, AudioGrant::Off);
    assert!(state.speaker_level.is_none());
    assert!(state.mic_gain.is_none());
    assert_eq!(state.schema_version, "v2");
}
