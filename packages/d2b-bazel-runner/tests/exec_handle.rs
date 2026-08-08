use std::{fs, path::Path};

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
