use std::{fs, path::PathBuf, process::Command};

#[test]
fn dependent_cannot_forge_registration_or_mint_admitted_session() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = repository_root
        .join(".scratch")
        .join(format!("bus-external-seals-{}", std::process::id()));
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local compiler scratch");

    for (test, expected) in [
        (
            "forge_registration",
            ["no `SessionRegistration` in the root", "error[E0432]"],
        ),
        (
            "clone_admitted",
            [
                "no method named `clone` found for struct `AuthenticatedComponentSession<C>`",
                "error[E0599]",
            ],
        ),
        (
            "forge_admission",
            [
                "expected `ComponentSessionAdmission`, found `ForeignAdmission`",
                "error[E0308]",
            ],
        ),
    ] {
        let output = Command::new(env!("CARGO"))
            .args([
                "check",
                "--quiet",
                "--locked",
                "--manifest-path",
                fixture.join("Cargo.toml").to_str().unwrap(),
                "--test",
                test,
            ])
            .env("CARGO_TARGET_DIR", scratch.join("target"))
            .env("TMPDIR", &temp)
            .output()
            .expect("run dependent compile-fail crate");
        let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");
        assert!(!output.status.success(), "{test} unexpectedly compiled");
        for diagnostic in expected {
            assert!(
                stderr.contains(diagnostic),
                "{test} did not produce {diagnostic:?}:\n{stderr}"
            );
        }
    }

    fs::remove_dir_all(&scratch).expect("remove repository-local compiler scratch");
}
