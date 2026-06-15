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
        panic!("failed to parse fixture {name} as {}: {err}", std::any::type_name::<T>())
    })
}

pub fn load_privileges_fixture_from_env() -> PrivilegesJson {
    parse_fixture("privileges.json")
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
    let manifest: ManifestV04 = parse_fixture("manifest.json");
    BundleResolver::from_artifacts(bundle, host, processes, manifest)
}
