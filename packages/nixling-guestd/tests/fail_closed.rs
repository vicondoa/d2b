use std::process::Command;

#[test]
fn binary_fails_closed_until_service_loop_lands() {
    let bin = env!("CARGO_BIN_EXE_nixling-guestd");

    let no_args = Command::new(bin).status().expect("run nixling-guestd");
    assert_eq!(no_args.code(), Some(78));

    let unknown = Command::new(bin)
        .arg("--unknown")
        .status()
        .expect("run nixling-guestd --unknown");
    assert_eq!(unknown.code(), Some(78));

    let version = Command::new(bin)
        .arg("--version")
        .status()
        .expect("run nixling-guestd --version");
    assert!(version.success());
}
