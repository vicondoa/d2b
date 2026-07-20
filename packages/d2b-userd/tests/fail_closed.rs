use std::process::Command;

#[test]
fn binary_requires_production_composition_and_activated_user_listener() {
    let bin = env!("CARGO_BIN_EXE_d2b-userd");

    let no_args = Command::new(bin).status().expect("run d2b-userd");
    assert_eq!(no_args.code(), Some(78));

    let unknown = Command::new(bin)
        .arg("--unknown")
        .status()
        .expect("run d2b-userd --unknown");
    assert_eq!(unknown.code(), Some(78));

    let version = Command::new(bin)
        .arg("--version")
        .status()
        .expect("run d2b-userd --version");
    assert!(version.success());
}
