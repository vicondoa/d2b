use std::{fs, path::PathBuf, process::Command};

#[test]
fn foreign_source_cannot_mint_committed_decision() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = repository_root.join(".scratch").join(format!(
        "controller-toolkit-external-seals-{}",
        std::process::id()
    ));
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local compiler scratch");

    let output = Command::new(env!("CARGO"))
        .args([
            "check",
            "--quiet",
            "--locked",
            "--manifest-path",
            fixture.join("Cargo.toml").to_str().unwrap(),
            "--test",
            "forge_committed_decision",
        ])
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .env("TMPDIR", &temp)
        .output()
        .expect("run dependent compile-fail crate");
    let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");

    assert!(!output.status.success(), "fixture unexpectedly compiled");
    assert!(
        !stderr.contains("error[E0432]"),
        "fixture failed on an unresolved import instead of the sealed boundary:\n{stderr}"
    );
    for diagnostic in [
        "error[E0451]",
        "fields `zone`, `resource_uid`, `generation`, `revision` and `operation_id` of struct `CommittedRevisionProof` are private",
    ] {
        assert!(
            stderr.contains(diagnostic),
            "fixture did not produce the expected sealed-boundary diagnostic {diagnostic:?}:\n{stderr}"
        );
    }

    fs::remove_dir_all(&scratch).expect("remove repository-local compiler scratch");
}
