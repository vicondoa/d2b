use std::process::Command;

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[test]
fn no_bootstrap_descriptor_fails_closed() {
    // Cargo guarantees CARGO_BIN_EXE_<bin> as a compile-time constant when
    // the integration test is compiled against the package's bin target
    // (the pattern the d2b CLI tests use); the runtime env-var fallback is
    // unreliable under `cargo test --workspace` invocation modes.
    let binary = env!("CARGO_BIN_EXE_d2b-provider-test-controller");
    let output = Command::new(binary)
        .output()
        .expect("spawn controller fixture");
    assert!(
        !output.status.success(),
        "controller without inherited fd10 must fail closed"
    );
}
