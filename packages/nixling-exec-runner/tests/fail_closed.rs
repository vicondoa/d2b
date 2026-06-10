use std::process::Command;

#[test]
fn binary_fails_closed_until_service_mode_lands() {
    let bin = env!("CARGO_BIN_EXE_nixling-exec-runner");

    let no_args = Command::new(bin).status().expect("run nixling-exec-runner");
    assert_eq!(no_args.code(), Some(78));

    let unknown = Command::new(bin)
        .arg("--unknown")
        .status()
        .expect("run nixling-exec-runner --unknown");
    assert_eq!(unknown.code(), Some(78));

    let version = Command::new(bin)
        .arg("--version")
        .status()
        .expect("run nixling-exec-runner --version");
    assert!(version.success());
}
