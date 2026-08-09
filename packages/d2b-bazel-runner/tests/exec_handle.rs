use std::{fs, path::Path, process::Command};

#[test]
fn adapter_delegates_once_to_the_dependency_leaf_consumer() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/exec_handle.rs");
    let source = fs::read_to_string(source_path).expect("adapter source");

    assert_eq!(
        source.matches("d2b_bazel_exec::execute_verified").count(),
        1,
        "the adapter has one exact typed-consumer delegation"
    );
    for forbidden in [
        "Command::new",
        "std::process::Command",
        "ExecutionBackend",
        "IMMUTABLE_SUPERVISOR_PATH",
        "d2b-bazel-exec-supervisor",
        "CARGO_BIN_EXE_",
        "RUNFILES",
        "worktree",
    ] {
        assert!(
            !source.contains(forbidden),
            "the adapter must not contain direct helper or path execution: {forbidden}"
        );
    }
}

#[test]
fn runner_library_exports_only_the_typed_adapter() {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let source = fs::read_to_string(source_path).expect("runner source");
    assert!(source.contains("pub use exec_handle::execute"));
    assert!(!source.contains("std::process::Command"));
    assert!(!source.contains("CARGO_BIN_EXE_"));
}

#[test]
fn execution_probe_accepts_a_clean_descriptor_set() {
    let output = Command::new(env!("CARGO_BIN_EXE_d2b-exec-probe"))
        .output()
        .expect("execution probe");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("probe output"),
        "D2B-BZLEXEC-PROBE status=closed\n"
    );
}
