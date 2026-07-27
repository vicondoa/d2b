use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
};

fn check_rejected(
    cargo: &str,
    manifest: &Path,
    target: &Path,
    temp: &Path,
    rustc_wrapper: &Path,
    cfg_test_marker: &Path,
    test: &str,
    expected: &[&str],
) {
    let output = Command::new(cargo)
        .args([
            "check",
            "--quiet",
            "--locked",
            "--all-features",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--test",
            test,
        ])
        .env("CARGO_TARGET_DIR", target)
        .env("D2B_CFG_TEST_MARKER", cfg_test_marker)
        .env("RUSTC_WRAPPER", rustc_wrapper)
        .env("TMPDIR", temp)
        .output()
        .expect("run dependent compile-fail crate");
    let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");

    assert!(!output.status.success(), "{test} unexpectedly compiled");
    for diagnostic in expected {
        assert!(
            stderr.contains(diagnostic),
            "{test} did not produce the expected privacy error {diagnostic:?}:\n{stderr}"
        );
    }
}

#[test]
fn dependent_cannot_mint_admission_or_session_capabilities() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = repository_root.join(".scratch").join(format!(
        "resource-api-external-seals-{}",
        std::process::id()
    ));
    let target = scratch.join("target");
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local compiler scratch");
    let rustc_wrapper = scratch.join("force-resource-api-cfg-test.sh");
    let cfg_test_marker = scratch.join("resource-api-cfg-test-active");
    fs::write(
        &rustc_wrapper,
        r#"#!/bin/sh
rustc="$1"
shift
previous=
for argument in "$@"; do
    if [ "$previous" = "--crate-name" ] && [ "$argument" = "d2b_resource_api" ]; then
        : > "$D2B_CFG_TEST_MARKER"
        exec "$rustc" --cfg test "$@"
    fi
    previous="$argument"
done
exec "$rustc" "$@"
"#,
    )
    .expect("write selective cfg(test) rustc wrapper");
    fs::set_permissions(&rustc_wrapper, fs::Permissions::from_mode(0o700))
        .expect("make selective cfg(test) rustc wrapper executable");

    let manifest = fixture.join("Cargo.toml");
    let cargo = env!("CARGO");
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        &rustc_wrapper,
        &cfg_test_marker,
        "forge_issuer",
        &["error[E0432]", "no `AdmissionIssuer` in the root"],
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        &rustc_wrapper,
        &cfg_test_marker,
        "forge_permit",
        &["error[E0432]", "no `AdmissionPermit` in the root"],
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        &rustc_wrapper,
        &cfg_test_marker,
        "forge_subject",
        &["error[E0599]", "no function or associated item named `new`"],
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        &rustc_wrapper,
        &cfg_test_marker,
        "private_admission_path",
        &["error[E0603]", "module `admission` is private"],
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        &rustc_wrapper,
        &cfg_test_marker,
        "private_test_issuer",
        &["error[E0603]", "module `identity` is private"],
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        &rustc_wrapper,
        &cfg_test_marker,
        "private_fields",
        &[
            "error[E0616]",
            "field `authority` of struct `AdmissionVerifier` is private",
            "field `authority` of struct `StoreIdentity` is private",
            "field `mutations` of struct `AdmittedMutation` is private",
            "field `claims` of struct `AuthenticatedSubjectContext` is private",
            "field `subject` of struct `TrustedRequest` is private",
        ],
    );
    assert!(
        cfg_test_marker.is_file(),
        "the resource API was not compiled under forced cfg(test)"
    );

    fs::remove_dir_all(&scratch).expect("remove repository-local compiler scratch");
}
