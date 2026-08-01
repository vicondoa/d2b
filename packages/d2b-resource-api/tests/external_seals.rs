use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash as _, Hasher as _},
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
};

/// A repository-local compiler scratch that persists between runs, keyed on the
/// compiler that produced it. Compiled artifacts are not portable across
/// compiler versions, so an unkeyed tree lets one toolchain poison another.
///
/// This tree is deliberately **not** under `.scratch/rust-test-cache`, which the
/// CI rust-cache carries between jobs. It measures 767 MB, and the Actions cache
/// is a hard repository-wide budget that is already fully subscribed; buying a
/// warm fixture here would evict entries whose cold rebuild costs far more than
/// this fixture does. The saving below is therefore a local one, and CI keeps
/// paying the cold build.
struct Scratch(PathBuf);

impl Scratch {
    fn new(path: PathBuf) -> Self {
        if std::env::var_os("D2B_EXTERNAL_SEALS_FRESH").is_some() && path.exists() {
            fs::remove_dir_all(&path).expect("discard repository-local compiler scratch");
        }
        fs::create_dir_all(&path).expect("create repository-local scratch");
        Self(path)
    }
}

/// Hash `rustc -vV` so a toolchain change lands in a different tree rather than
/// reusing artifacts the new compiler cannot read.
fn toolchain_cache_key() -> String {
    let version = Command::new(std::env::var("RUSTC").as_deref().unwrap_or("rustc"))
        .arg("-vV")
        .output()
        .expect("query the active rustc version");
    assert!(
        version.status.success(),
        "rustc -vV failed; a scratch tree keyed on an unknown toolchain could be reused across \
         incompatible compilers"
    );
    let mut hasher = DefaultHasher::new();
    version.stdout.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Force the next `cargo check` to recompile `d2b-resource-api`, whatever the
/// tree already holds, by discarding its fingerprints. The dependencies below it
/// stay warm, which is the whole saving.
///
/// This exists because the seal's proof is that the crate compiled under forced
/// `cfg(test)`, recorded by the rustc shim writing a marker. A warm tree would
/// otherwise skip that compile, and the marker would never be written - measured
/// directly: warm without this call leaves the marker absent, warm with it
/// present, and a cold tree writes it as it always did.
///
/// So the failure mode is fail-closed by construction. If this ever stops
/// forcing the compile - cargo relocates its fingerprints, renames the unit, or
/// the glob simply stops matching - the marker is absent and the assertion at
/// the end of the test fails. It cannot cause the seal to pass without proof.
/// Matching nothing is not an error: on a cold tree there is nothing to discard
/// and the compile happens regardless.
fn force_resource_api_recompile(target: &Path) {
    let fingerprints = target.join("debug/.fingerprint");
    let entries = match fs::read_dir(&fingerprints) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!(
            "could not read {} to force a fresh cfg(test) compile: {error}",
            fingerprints.display()
        ),
    };
    for entry in entries {
        let entry = entry.expect("read a cargo fingerprint directory entry");
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("d2b-resource-api"))
        {
            // Fail closed: a fingerprint that survives leaves the unit fresh, and
            // the marker assertion would then report an absent compile as a seal
            // failure. Surface the real cause here instead.
            fs::remove_dir_all(entry.path()).unwrap_or_else(|error| {
                panic!(
                    "could not discard the cargo fingerprint at {}; the forced cfg(test) compile \
                     would be skipped: {error}",
                    entry.path().display()
                )
            });
        }
    }
}

struct CompileFailHarness<'a> {
    cargo: &'a str,
    manifest: &'a Path,
    target: &'a Path,
    temp: &'a Path,
    rustc_wrapper: &'a Path,
    cfg_test_marker: &'a Path,
}

impl CompileFailHarness<'_> {
    fn check_rejected(&self, test: &str, expected: &[&str]) {
        let output = Command::new(self.cargo)
            .args([
                "check",
                "--quiet",
                "--locked",
                "--all-features",
                "--manifest-path",
                self.manifest.to_str().unwrap(),
                "--test",
                test,
            ])
            .env("CARGO_TARGET_DIR", self.target)
            .env("D2B_CFG_TEST_MARKER", self.cfg_test_marker)
            // The outer gate rejects warnings, but this fixture deliberately
            // forces cfg(test) onto a dependency without building its unit-test
            // harness. That makes the dependency's test helpers look unused.
            // Cap lints for this diagnostic-only compile so those expected
            // warnings cannot mask the privacy error the seal is asserting.
            .env("CARGO_ENCODED_RUSTFLAGS", "--cap-lints\u{1f}allow")
            // This harness owns RUSTC_WRAPPER: it must be the selective
            // cfg(test) shim below, never the repository's caching wrapper,
            // whose client or server can exit nonzero under concurrent cargo
            // invocations and turn a load-bearing seal assertion into a
            // spurious failure. Clear the other wrapper spellings for the same
            // reason, so an inherited workspace or config-env wrapper cannot
            // layer that contention back on top of the shim.
            .env("RUSTC_WRAPPER", self.rustc_wrapper)
            .env("RUSTC_WORKSPACE_WRAPPER", "")
            .env("CARGO_BUILD_RUSTC_WRAPPER", "")
            .env("TMPDIR", self.temp)
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
}

#[test]
fn dependent_cannot_mint_admission_or_session_capabilities() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = Scratch::new(repository_root.join(".scratch").join(format!(
        "resource-api-external-seals-{}",
        toolchain_cache_key()
    )));
    let scratch = scratch.0.clone();
    let target = scratch.join("target");
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local compiler scratch");
    // Both paths must be stable across runs. Cargo fingerprints RUSTC_WRAPPER,
    // so a per-run wrapper path would invalidate the whole tree and give back
    // the cold build this cache exists to avoid.
    let rustc_wrapper = scratch.join("force-resource-api-cfg-test.sh");
    let cfg_test_marker = scratch.join("resource-api-cfg-test-active");
    // Discard any marker a previous run left, so the assertion at the end of
    // this test can only be satisfied by a compile that happened during it.
    match fs::remove_file(&cfg_test_marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!(
            "could not discard the cfg(test) marker at {}; a stale one would satisfy the seal \
             without a compile: {error}",
            cfg_test_marker.display()
        ),
    }
    force_resource_api_recompile(&target);
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
    let harness = CompileFailHarness {
        cargo,
        manifest: &manifest,
        target: &target,
        temp: &temp,
        rustc_wrapper: &rustc_wrapper,
        cfg_test_marker: &cfg_test_marker,
    };
    harness.check_rejected(
        "forge_issuer",
        &["error[E0432]", "no `AdmissionIssuer` in the root"],
    );
    harness.check_rejected(
        "forge_permit",
        &["error[E0432]", "no `AdmissionPermit` in the root"],
    );
    harness.check_rejected(
        "forge_subject",
        // rustc 1.97 rewords E0599 for an absent associated item from "no
        // function or associated item named" to "no associated function or
        // constant named". Match the shorter stable substring both spellings
        // share, so the seal keeps asserting that `new` is unreachable without
        // re-breaking on the next rewording.
        &["error[E0599]", "named `new`"],
    );
    harness.check_rejected(
        "private_admission_path",
        &["error[E0603]", "module `admission` is private"],
    );
    harness.check_rejected(
        "private_test_issuer",
        &["error[E0603]", "module `identity` is private"],
    );
    harness.check_rejected(
        "private_fields",
        &[
            "error[E0616]",
            "field `mutations` of struct `AdmittedMutation` is private",
            "field `claims` of struct `AuthenticatedSubjectContext` is private",
            "field `subject` of struct `TrustedRequest` is private",
        ],
    );
    harness.check_rejected(
        "shared_store_tokens",
        &[
            "error[E0432]",
            "no `AdmissionVerifier` in the root",
            "no `StoreIdentity` in the root",
        ],
    );
    assert!(
        cfg_test_marker.is_file(),
        "the resource API was not compiled under forced cfg(test)"
    );
}
