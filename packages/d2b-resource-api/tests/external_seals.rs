use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn check_rejected(
    cargo: &str,
    manifest: &Path,
    target: &Path,
    temp: &Path,
    binary: &str,
    expected: &str,
) {
    let output = Command::new(cargo)
        .args([
            "check",
            "--quiet",
            "--locked",
            "--manifest-path",
            manifest.to_str().unwrap(),
            "--bin",
            binary,
        ])
        .env("CARGO_TARGET_DIR", target)
        .env("RUSTC_WRAPPER", "")
        .env("TMPDIR", temp)
        .output()
        .expect("run dependent compile-fail crate");
    let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");

    assert!(!output.status.success(), "{binary} unexpectedly compiled");
    assert!(
        stderr.contains(expected),
        "{binary} did not produce the expected privacy error:\n{stderr}"
    );
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

    let manifest = fixture.join("Cargo.toml");
    let cargo = env!("CARGO");
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        "forge_issuer",
        "no `AdmissionIssuer` in the root",
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        "forge_permit",
        "no `AdmissionPermit` in the root",
    );
    check_rejected(
        cargo,
        &manifest,
        &target,
        &temp,
        "forge_subject",
        "no function or associated item named `new`",
    );

    fs::remove_dir_all(&scratch).expect("remove repository-local compiler scratch");
}
