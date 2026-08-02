use d2b_provider_device_tpm::{
    BinaryKind, SignedBinaryRef, StateDirIntent, StateDirectoryToken, StateOwnerToken, SwtpmArgv,
    SwtpmArgvError, SwtpmSettings, TamperMarkerToken, TpmStateObservation, TpmStateObservationKind,
    TpmStateValidationError, validate_start_ticket,
};

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

#[test]
fn argv_shape_is_path_free_and_byte_stable() {
    let argv = SwtpmArgv::for_settings(SwtpmSettings::default()).unwrap();
    assert_eq!(
        argv.args(),
        [
            "swtpm",
            "socket",
            "--tpm2",
            "--tpmstate",
            "<state-dir>",
            "--ctrl",
            "<ctrl-socket>",
            "--server",
            "<server-socket>",
            "--flags",
            "startup-clear",
            "--log",
            "file=<state-dir>/swtpm.log,level=<log-level>",
            "--pid",
            "<state-dir>/swtpm.pid",
            "--daemon=false"
        ]
    );
    assert_eq!(
        SwtpmArgv::flush_args(),
        ["swtpm_ioctl", "-i", "--unix", "<ctrl-socket>"]
    );
}

#[test]
fn start_ticket_is_bound_to_the_opaque_state_intent() {
    let intent = StateDirIntent::new(
        StateDirectoryToken::from_core([1; 32]),
        TamperMarkerToken::from_core([2; 32]),
        StateOwnerToken::from_core([3; 16]),
    );
    let binary = SignedBinaryRef::from_core(BinaryKind::Swtpm, [4; 32]);
    assert_eq!(
        validate_start_ticket(&intent, intent.directory(), &binary),
        Ok(())
    );
    assert_eq!(
        validate_start_ticket(&intent, &StateDirectoryToken::from_core([9; 32]), &binary),
        Err(SwtpmArgvError::TicketIntentMismatch)
    );
}

#[test]
fn missing_prior_marker_fails_closed() {
    let intent = StateDirIntent::new(
        StateDirectoryToken::from_core([1; 32]),
        TamperMarkerToken::from_core([2; 32]),
        StateOwnerToken::from_core([3; 16]),
    );
    let error = intent
        .validate(&TpmStateObservation::from_core(
            TpmStateObservationKind::MissingMarker,
        ))
        .unwrap_err();
    assert_eq!(
        error,
        TpmStateValidationError::PreviouslyProvisionedStateMissing
    );
    assert_eq!(error.code(), "previously-provisioned-swtpm-state-missing");
}
