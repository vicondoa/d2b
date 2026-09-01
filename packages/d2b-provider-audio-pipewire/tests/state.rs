use d2b_provider_audio_pipewire::{
    AudioGrant, AudioPolicyState, LevelPercent, read_audio_state_locked, write_audio_state_locked,
};

#[test]
fn ofd_read_write_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lock_path = dir.path().join("audio-test.lock");
    let state_path = dir.path().join("state").join("audio-state.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();

    let state = AudioPolicyState::default_v2()
        .with_mic(AudioGrant::On)
        .with_speaker(AudioGrant::Off)
        .with_speaker_level(LevelPercent::new(75).unwrap());
    write_audio_state_locked(&lock_path, &state_path, &state).expect("write state");

    let read_back = read_audio_state_locked(&lock_path, &state_path).expect("read state");
    assert_eq!(read_back.mic, AudioGrant::On);
    assert_eq!(read_back.speaker, AudioGrant::Off);
    assert_eq!(read_back.speaker_level.map(|level| level.get()), Some(75));
}

#[test]
fn ofd_missing_state_returns_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lock_path = dir.path().join("audio-test.lock");
    let state_path = dir.path().join("audio-state.json");

    let state = read_audio_state_locked(&lock_path, &state_path).expect("read missing");
    assert_eq!(state, AudioPolicyState::default_v2());
}

#[test]
fn write_is_atomic_rename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lock_path = dir.path().join("audio.lock");
    let state_path = dir.path().join("audio-state.json");

    write_audio_state_locked(&lock_path, &state_path, &AudioPolicyState::default_v2()).unwrap();

    assert!(!dir.path().join("audio-state.json.tmp").exists());
}
