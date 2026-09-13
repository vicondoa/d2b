use d2b_provider_device_tpm::{SwtpmArgvError, SwtpmSettings};

#[test]
fn settings_are_strict_and_bounded() {
    let settings: SwtpmSettings = serde_json::from_str(r#"{"logLevel":20}"#).unwrap();
    assert_eq!(settings, SwtpmSettings::default());
    assert!(
        serde_json::from_str::<SwtpmSettings>(r#"{"logLevel":20,"startupClear":true}"#).is_err()
    );
    assert_eq!(
        SwtpmSettings { log_level: 0 }.validate(),
        Err(SwtpmArgvError::LogLevelOutOfRange)
    );
}
