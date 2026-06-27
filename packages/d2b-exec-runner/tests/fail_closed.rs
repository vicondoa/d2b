use std::process::Command;

/// The binary fails closed on unknown/no-arg invocations and exposes only the
/// `--version` and `--serve-exec --slot NN` surfaces.
#[test]
fn binary_fails_closed_on_unsupported_invocations() {
    let bin = env!("CARGO_BIN_EXE_d2b-exec-runner");

    let no_args = Command::new(bin).status().expect("run d2b-exec-runner");
    assert_eq!(no_args.code(), Some(78));

    let unknown = Command::new(bin)
        .arg("--unknown")
        .status()
        .expect("run d2b-exec-runner --unknown");
    assert_eq!(unknown.code(), Some(78));

    let version = Command::new(bin)
        .arg("--version")
        .status()
        .expect("run d2b-exec-runner --version");
    assert!(version.success());
}

/// `--serve-exec` requires a `--slot` in range; a missing or out-of-range slot
/// is a usage error (64), and the binary never proceeds without a slot.
#[test]
fn serve_exec_rejects_missing_or_out_of_range_slot() {
    let bin = env!("CARGO_BIN_EXE_d2b-exec-runner");

    let missing_slot = Command::new(bin)
        .arg("--serve-exec")
        .status()
        .expect("run d2b-exec-runner --serve-exec");
    assert_eq!(missing_slot.code(), Some(64));

    let out_of_range = Command::new(bin)
        .args(["--serve-exec", "--slot", "99"])
        .status()
        .expect("run d2b-exec-runner --serve-exec --slot 99");
    assert_eq!(out_of_range.code(), Some(64));

    let non_numeric = Command::new(bin)
        .args(["--serve-exec", "--slot", "xx"])
        .status()
        .expect("run d2b-exec-runner --serve-exec --slot xx");
    assert_eq!(non_numeric.code(), Some(64));
}
