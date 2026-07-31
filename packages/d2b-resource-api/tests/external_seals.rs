use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
};

/// Owns the repository-local compiler scratch so it is removed on every exit
/// path, including a panicking assertion. Removing it only on success leaves a
/// uniquely-named target tree behind whenever cargo fails, and a repeatedly
/// failing gate accumulates them until it trips the disk-space preflight.
struct ScratchGuard(PathBuf);

impl ScratchGuard {
    /// Drop does not run when the process is killed by a signal - nextest's
    /// slow-timeout terminate, SIGKILL and OOM all skip it - so a tree can
    /// outlive its run and be adopted by a later one that reuses the PID. That
    /// would hand the seal a warm target dir plus a stale
    /// `resource-api-cfg-test-active` marker, and `cfg_test_marker.is_file()`
    /// would pass without a compile having happened. Remove any existing tree
    /// before creating, so adoption cannot occur.
    fn new(path: PathBuf) -> Self {
        // Fail closed, matching Scratch::ephemeral in d2b-bus. Swallowing the
        // error would let a tree that could not be removed (EACCES on a
        // mode-changed entry, EBUSY, a racing writer) be adopted anyway, since
        // create_dir_all succeeds on an existing directory - reinstating the
        // very hole this guard closes.
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!(
                "could not discard a stranded scratch tree at {}; adopting it would let the \
                 cfg(test) marker pass without a compile: {error}",
                path.display()
            ),
        }
        Self(path)
    }
}

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
    // Per-process and uncached, deliberately: the cfg_test_marker assertion
    // below proves the crate was compiled under forced cfg(test), and a warm
    // target dir would skip that compile and leave a stale marker satisfying
    // the assertion without proving anything.
    //
    // Owned by a guard so the tree is removed on every exit path. Removing it
    // only on success strands a uniquely-named multi-gigabyte tree per failing
    // run, and preflight-disk-space.sh fails the wave below 10 GiB free.
    let scratch_guard = ScratchGuard::new(repository_root.join(".scratch").join(format!(
        "resource-api-external-seals-{}",
        std::process::id()
    )));
    let scratch = scratch_guard.0.clone();
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

    // ScratchGuard removes the tree on drop, including on a panicking
    // assertion above, so there is no explicit cleanup here.
}
