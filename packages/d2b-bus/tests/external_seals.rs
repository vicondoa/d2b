use std::{fs, path::PathBuf, process::Command};

#[test]
fn dependent_cannot_forge_registration_or_mint_admitted_session() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = Scratch::new(
        repository_root
            .join(".scratch")
            .join(format!("bus-external-seals-{}", std::process::id())),
    );
    let temp = scratch.path().join("tmp");
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
            ["no `SessionAuthority` in the root", "error[E0432]"],
        ),
        (
            "forge_native_authority",
            ["no `NativeSessionAuthority` in the root", "error[E0432]"],
        ),
        (
            "inject_unix_subject",
            ["no `UnixSubjectConfig` in the root", "error[E0432]"],
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
            .env("CARGO_TARGET_DIR", scratch.path().join("target"))
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

    for (mutation, expected_type) in [
        (
            "component-session-admission-clone",
            "ComponentSessionAdmission",
        ),
        (
            "component-session-admission-default",
            "ComponentSessionAdmission",
        ),
        ("verified-unix-peer-clone", "VerifiedUnixPeer"),
        ("verified-unix-peer-default", "VerifiedUnixPeer"),
        ("session-acceptor-clone", "SessionAcceptor<C>"),
        ("session-acceptor-default", "SessionAcceptor<C>"),
        (
            "authenticated-component-session-clone",
            "AuthenticatedComponentSession<C>",
        ),
        (
            "authenticated-component-session-default",
            "AuthenticatedComponentSession<C>",
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
                "capability_trait_mutations",
            ])
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                format!("--cfg\u{1f}d2b_capability_trait_mutation=\"{mutation}\""),
            )
            .env("CARGO_TARGET_DIR", scratch.path().join("target"))
            .env("TMPDIR", &temp)
            .output()
            .expect("run capability trait compile-fail mutation");
        let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");
        assert!(!output.status.success(), "{mutation} unexpectedly compiled");
        for diagnostic in [
            "error[E0283]",
            "multiple `impl`s satisfying",
            expected_type,
            "AmbiguousIfImpl",
        ] {
            assert!(
                stderr.contains(diagnostic),
                "{mutation} did not produce {diagnostic:?}:\n{stderr}"
            );
        }
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(path: PathBuf) -> Self {
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale repository-local scratch");
        }
        fs::create_dir_all(&path).expect("create repository-local scratch");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
