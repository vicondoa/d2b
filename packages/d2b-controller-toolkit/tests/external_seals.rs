use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash as _, Hasher as _},
    path::{Path, PathBuf},
    process::Command,
};

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
    // Hash rather than embed: `rustc -vV` is multi-line and runs past 200
    // characters, which overflows NAME_MAX once a prefix is added. A digest
    // keeps the commit hash and host triple participating at fixed width.
    let mut hasher = DefaultHasher::new();
    version.trim().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// A reusable repository-local compiler scratch tree.
///
/// This test drives several `cargo check` invocations against the compile-fail
/// fixture crate. The tree used to be per-process and deleted on drop, so every
/// run recompiled the fixture's whole dependency graph from cold.
///
/// The stable path also subsumes the reason the drop guard existed. A
/// uniquely-named tree removed only on success accumulates one leftover per
/// failed run until it fills the disk; a single stable tree is instead adopted
/// and reused by the next run, so a repeatedly failing gate costs one directory
/// rather than one per attempt.
///
/// Reuse is sound: Cargo owns staleness through its own fingerprints, and the
/// assertions are about compiler diagnostics Cargo re-produces whenever an
/// input changes. Cargo does not cache failed builds, so each compile-fail case
/// still genuinely recompiles the fixture; what survives is the dependency
/// graph beneath it.
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

    fn path(&self) -> &Path {
        &self.0
    }
}

#[test]
fn foreign_source_cannot_mint_committed_decision() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/external-seals");
    let scratch = repository_root
        .join(".scratch/rust-test-cache")
        .join(format!(
            "controller-toolkit-external-seals-{}",
            toolchain_cache_key()
        ));
    let scratch = Scratch::new(scratch);
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
            "forge_committed_decision",
        ])
        .env("CARGO_TARGET_DIR", scratch.path().join("target"))
        .env("TMPDIR", &temp)
        // Compile the fixture without any rustc wrapper. The repository config
        // sets a caching wrapper, whose client or server can exit nonzero under
        // concurrent cargo invocations; that failure is indistinguishable from
        // the fixture failing for the wrong reason, so it turns a load-bearing
        // seal assertion into a spurious failure. A compilation that is
        // expected to fail gains nothing from a compiler cache anyway. Clear
        // every wrapper spelling, not just RUSTC_WRAPPER, so an inherited
        // workspace or config-env wrapper cannot reintroduce the contention.
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env("CARGO_BUILD_RUSTC_WRAPPER", "")
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
}
