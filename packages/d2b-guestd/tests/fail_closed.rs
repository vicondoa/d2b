use std::process::Command;

#[test]
fn binary_requires_explicit_service_credentials() {
    let bin = env!("CARGO_BIN_EXE_d2b-guestd");

    let no_args = Command::new(bin).status().expect("run d2b-guestd");
    assert_eq!(no_args.code(), Some(78));

    let unknown = Command::new(bin)
        .arg("--unknown")
        .status()
        .expect("run d2b-guestd --unknown");
    assert_eq!(unknown.code(), Some(78));

    let missing_credential = Command::new(bin)
        .args(["--serve", "--vm-id", "corp-vm"])
        .status()
        .expect("run d2b-guestd --serve without credentials");
    assert_eq!(missing_credential.code(), Some(78));

    let version = Command::new(bin)
        .arg("--version")
        .status()
        .expect("run d2b-guestd --version");
    assert!(version.success());
}
