use std::{env, fs, path::PathBuf};

use nixling_core::bundle::Bundle;
use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::host::HostJson;
use nixling_core::manifest_v04::ManifestV04;
use nixling_core::privileges::PrivilegesJson;
use nixling_core::processes::ProcessesJson;

fn fixtures_dir() -> PathBuf {
    let fixtures = env::var_os("NL_FIXTURES")
        .unwrap_or_else(|| panic!("NL_FIXTURES must point to the fixture-smoke output directory"));
    PathBuf::from(fixtures)
}

fn read_fixture(name: &str) -> String {
    let path = fixtures_dir().join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture at {}: {err}", path.display()))
}

fn parse_fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let json = read_fixture(name);
    serde_json::from_str(&json).unwrap_or_else(|err| {
        panic!(
            "failed to parse fixture {name} as {}: {err}",
            std::any::type_name::<T>()
        )
    })
}

pub fn load_privileges_fixture_from_env() -> PrivilegesJson {
    parse_fixture("privileges.json")
}

/// Load the rendered public manifest (`manifest.json` == `vms.json`) as an
/// untyped `serde_json::Value` for the world-readable / opaque-key-id static
/// invariants (which traverse arbitrary scalar fields).
pub fn load_manifest_value_from_env() -> serde_json::Value {
    parse_fixture("manifest.json")
}

/// The feature-rich `fixture-smoke-full` output dir (NL_FIXTURES_FULL), or
/// `None` when unset — e.g. the plain `cargo test` pass, or a non-x86_64 host
/// where the graphics platform gate makes the fixture unavailable. Per-role
/// minijail-validator contract tests that need feature-specific profiles
/// (gpu/swtpm/audio/video/usbip/vsock-relay/wayland-proxy/otel-host-bridge)
/// skip cleanly when this is `None`.
fn full_fixtures_dir() -> Option<PathBuf> {
    env::var_os("NL_FIXTURES_FULL").map(PathBuf::from)
}

fn read_full_fixture(dir: &std::path::Path, name: &str) -> String {
    let path = dir.join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read full fixture at {}: {err}", path.display()))
}

/// Reconstruct a `BundleResolver` from the feature-rich `fixture-smoke-full`
/// artifacts (NL_FIXTURES_FULL), or `None` when that fixture is unavailable
/// (the caller should skip). Mirrors [`load_bundle_resolver_from_env`].
pub fn load_full_bundle_resolver_from_env() -> Option<BundleResolver> {
    let dir = full_fixtures_dir()?;
    let bundle: Bundle = serde_json::from_str(&read_full_fixture(&dir, "bundle.json"))
        .unwrap_or_else(|err| panic!("full bundle.json parse: {err}"));
    let host: HostJson = serde_json::from_str(&read_full_fixture(&dir, "host.json"))
        .unwrap_or_else(|err| panic!("full host.json parse: {err}"));
    let processes: ProcessesJson = serde_json::from_str(&read_full_fixture(&dir, "processes.json"))
        .unwrap_or_else(|err| panic!("full processes.json parse: {err}"));
    let manifest_bytes = read_full_fixture(&dir, "manifest.json");
    let manifest = ManifestV04::from_slice(manifest_bytes.as_bytes()).unwrap_or_else(|err| {
        panic!("full manifest.json failed ManifestV04::from_slice (version gate): {err:?}")
    });
    Some(BundleResolver::from_artifacts(
        bundle, host, processes, manifest,
    ))
}

/// Reconstruct a `BundleResolver` from the rendered fixture-smoke artifacts
/// (bundle/host/processes/manifest JSON), bypassing the on-disk
/// integrity/mode/uid verification `BundleResolver::load` performs (the
/// fixture lives in the read-only Nix store, not a 0640 root-owned bundle
/// dir). `from_artifacts` takes the already-parsed DTOs, so the manifest
/// version is whatever the fixture renders (currently MANIFEST_VERSION_CURRENT),
/// not a stale hard-coded test fixture.
pub fn load_bundle_resolver_from_env() -> BundleResolver {
    let bundle: Bundle = parse_fixture("bundle.json");
    let host: HostJson = parse_fixture("host.json");
    let processes: ProcessesJson = parse_fixture("processes.json");
    // Parse the manifest via the production `ManifestV04::from_slice`, which
    // enforces MANIFEST_VERSION_CURRENT — generic serde (parse_fixture) would
    // accept a stale rendered manifest that `BundleResolver::load` rejects,
    // letting the contract test pass on a version the daemon/broker refuse.
    let manifest_bytes = read_fixture("manifest.json");
    let manifest = ManifestV04::from_slice(manifest_bytes.as_bytes()).unwrap_or_else(|err| {
        panic!("manifest.json fixture failed ManifestV04::from_slice (version gate): {err:?}")
    });
    BundleResolver::from_artifacts(bundle, host, processes, manifest)
}

// ---------------------------------------------------------------------------
// Repo-file access for the policy/source/doc-lint layer (the H-group gates).
//
// This crate is excluded from the hermetic Nix sandbox workspace build and
// runs only from tests/rust-workspace-checks.sh against the real checkout, so
// reading repo files relative to CARGO_MANIFEST_DIR is sound here (it is NOT
// sound for crates built in the sandbox).
// ---------------------------------------------------------------------------

/// Absolute path to the repository root (two levels up from this crate).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonicalize repo root from CARGO_MANIFEST_DIR")
}

/// Read a repo-relative file to a string, panicking with a clear message when
/// absent (a policy lint asserting a file's content must fail, not skip, if the
/// file is missing).
pub fn read_repo_file(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("policy-lint: cannot read {}: {err}", path.display()))
}

/// Whether a repo-relative path exists.
pub fn repo_path_exists(rel: &str) -> bool {
    repo_root().join(rel).exists()
}
