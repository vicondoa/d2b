use d2b_provider_device_tpm::{SwtpmArgv, SwtpmArgvError, SwtpmSettings};

#[test]
fn settings_are_strict_and_bounded() {
    let settings: SwtpmSettings =
        serde_json::from_str(r#"{"logLevel":20,"startupClear":true}"#).unwrap();
    assert_eq!(settings, SwtpmSettings::default());
    assert!(
        serde_json::from_str::<SwtpmSettings>(
            r#"{"logLevel":20,"startupClear":true,"stateDirPath":"/tmp/x"}"#
        )
        .is_err()
    );
    assert_eq!(
        SwtpmSettings {
            log_level: 0,
            startup_clear: true
        }
        .validate(),
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
            "<state-dir>/swtpm.log",
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
