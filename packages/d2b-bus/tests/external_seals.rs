use std::{fs, path::PathBuf, process::Command};

#[test]
fn dependent_cannot_forge_registration_or_mint_admitted_session() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = Scratch::new(
        repository_root
            .join(".scratch")
            .join(format!("bus-external-seals-{}", toolchain_cache_key())),
    );
    let temp = scratch.path().join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local compiler scratch");

    let output = Command::new(env!("CARGO"))
        .args([
            "check",
            "--quiet",
            "--locked",
            "--manifest-path",
            fixture.join("Cargo.toml").to_str().unwrap(),
            "--test",
            "downstream_from_existing_authority",
        ])
        .env("CARGO_TARGET_DIR", scratch.path().join("target"))
        .env("TMPDIR", &temp)
        .output()
        .expect("run downstream From compile-pass fixture");
    assert!(
        output.status.success(),
        "downstream local-input From impl did not compile; Cargo status {}",
        output.status
    );

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
        (
            "downstream_from_fabrication",
            [
                "field `identity` of struct `ComponentSessionAdmission` is private",
                "error[E0451]",
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
            .env("CARGO_TARGET_DIR", scratch.path().join("target"))
            .env("TMPDIR", &temp)
            .output()
            .expect("run dependent compile-fail crate");
        let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");
        assert!(!output.status.success(), "{test} unexpectedly compiled");
        for diagnostic in expected {
            assert!(
                stderr.contains(diagnostic),
                "{test} did not produce required compiler diagnostic {diagnostic:?}; \
                 raw Cargo stderr is redacted"
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
        (
            "component-session-admission-from-unit",
            "ComponentSessionAdmission",
        ),
        ("verified-unix-peer-clone", "VerifiedUnixPeer"),
        ("verified-unix-peer-default", "VerifiedUnixPeer"),
        ("verified-unix-peer-from-unit", "VerifiedUnixPeer"),
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
            "CapabilityMustNotImplementCloneCopyDefaultOrFrom",
        ] {
            assert!(
                stderr.contains(diagnostic),
                "{mutation} did not produce required compiler diagnostic {diagnostic:?}; \
                 raw Cargo stderr is redacted"
            );
        }
    }
}

/// A directory-safe token identifying the toolchain driving the nested cargo
/// invocations. Compiled artifacts are not portable across compiler versions,
/// and the gate provisions its own pinned toolchain while a developer shell
/// commonly has a different one, so each gets its own cache tree rather than
/// corrupting a shared one.
fn toolchain_cache_key() -> String {
    let version = Command::new("rustc")
        .arg("-vV")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .expect(
            "rustc -vV must identify the compiler: a cache shared between two \
             unidentified toolchains is the corruption this key prevents",
        );
    let token: String = version
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    token.trim_matches('-').to_owned()
}

/// A reusable repository-local compiler scratch tree.
///
/// This test drives roughly ten `cargo check` invocations against the
/// compile-fail fixture crate. They already share one target directory within a
/// run, but the tree used to be per-process and deleted on drop, so every run
/// recompiled the fixture's whole dependency graph - d2b-bus, d2b-session,
/// d2b-session-unix and everything beneath them - from cold. That made this the
/// slowest test in the workspace and, once the suite moved to cargo-nextest,
/// the critical path bounding the entire test phase.
///
/// Reuse is sound: Cargo owns staleness through its own fingerprints, and the
/// assertions are about compiler diagnostics that Cargo re-produces whenever an
/// input changes. Cargo does not cache failed builds, so each compile-fail case
/// still genuinely recompiles the fixture crate; what survives is the
/// dependency graph beneath it, which is the bulk of the cost.
///
/// Unlike the rustdoc cache in public_mint_surface there is no output-collision
/// hazard here, because these invocations produce diagnostics rather than a
/// shared HTML tree.
///
/// A stable path also makes this leak-proof: an interrupted run leaves a
/// directory the next run adopts, rather than stranding a per-process tree
/// nothing will collect.
///
/// Set `D2B_EXTERNAL_SEALS_FRESH=1` to discard the cache and compile cold.
struct Scratch(PathBuf);

impl Scratch {
    fn new(path: PathBuf) -> Self {
        if std::env::var_os("D2B_EXTERNAL_SEALS_FRESH").is_some() && path.exists() {
            fs::remove_dir_all(&path).expect("discard repository-local compiler scratch");
        }
        fs::create_dir_all(&path).expect("create repository-local scratch");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
