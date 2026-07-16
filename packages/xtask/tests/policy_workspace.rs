#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const CONTRACTS_CRATE: &str = "d2b-contracts";
const FOCUSED_POLICY_PACKAGES: &[&str] = &["d2b-priv-broker", "d2b-guest-shell-runner"];
const V2_FOUNDATION_CRATES: &[&str] = &[
    "d2b-client",
    "d2b-provider",
    "d2b-provider-toolkit",
    "d2b-session",
    "d2b-session-unix",
    "d2b-state",
];
const IMPLEMENTATION_CRATES: &[&str] = &[
    "d2b-provider-aca",
    "d2b-provider-host",
    "d2b-provider-relay",
    "d2b-realm-codec-protobuf",
    "d2b-session-unix",
];
const W4_PROVIDER_CRATES: &[&str] = &[
    "d2b-provider-audio-pipewire-vhost-user",
    "d2b-provider-credential-entra",
    "d2b-provider-credential-managed-identity",
    "d2b-provider-credential-secret-service",
    "d2b-provider-device-host-mediated",
    "d2b-provider-display-wayland",
    "d2b-provider-infrastructure-azure-vm",
    "d2b-provider-network-local-realm",
    "d2b-provider-observability-local",
    "d2b-provider-runtime-azure-container-apps",
    "d2b-provider-runtime-azure-vm",
    "d2b-provider-runtime-local",
    "d2b-provider-storage-local",
    "d2b-provider-substrate-host",
    "d2b-provider-transport-azure-relay",
    "d2b-provider-transport-local",
];
const W4_SUPPORT_CRATE: &str = "d2b-azure-vm-fake-sdk";
const W4_UNAVAILABLE_PROVIDER_SCAFFOLDS: &[&str] = &[
    "d2b-provider-infrastructure-azure-vm",
    "d2b-provider-runtime-azure-vm",
];
const NON_PRODUCTION_BINARY_PACKAGES: &[&str] = &["d2b-core-fuzz"];
const REQUIRED_SHIPPED_PRODUCTION_PACKAGES: &[&str] = &[
    "d2b",
    "d2b-clipd",
    "d2b-exec-runner",
    "d2b-gateway",
    "d2b-gateway-runtime",
    "d2b-guest-shell-runner",
    "d2b-guestd",
    "d2b-host",
    "d2b-host-activation-helper",
    "d2b-notify",
    "d2b-priv-broker",
    "d2b-unsafe-local-helper",
    "d2b-userd",
    "d2b-wayland-proxy",
    "d2bd",
];
const NON_RUST_FLAKE_PACKAGE_OUTPUTS: &[&str] = &[
    "cargo-semver-checks",
    "cargo-udeps-nightly",
    "completions",
    "gh",
    "git-town",
    "manpages",
    "signoz",
    "signozOtelCollector",
    "signozSchemaMigrator",
];
const PROVIDER_INTEGRATION_FILES: &[&str] = &[
    "docs/reference/daemon-api.md",
    "docs/reference/manifest-bundle.md",
    "docs/reference/schemas/v2/bundle.json",
    "docs/reference/schemas/v2/bundle.md",
    "docs/reference/schemas/v2/provider-registry-v2.json",
    "docs/reference/schemas/v2/provider-registry-v2.md",
    "flake.nix",
    "nixos-modules/assertions.nix",
    "nixos-modules/bundle-artifacts.nix",
    "nixos-modules/bundle.nix",
    "nixos-modules/default.nix",
    "nixos-modules/processes-json.nix",
    "nixos-modules/provider-registry-v2-json.nix",
    "packages/Cargo.lock",
    "packages/Cargo.toml",
    "packages/d2b-contract-tests/tests/realm_workload_schema_contract.rs",
    "packages/d2b-contracts/src/lib.rs",
    "packages/d2b-contracts/src/provider_registry_v2.rs",
    "packages/d2b-core/fuzz/src/bin/core.rs",
    "packages/d2b-core/src/bundle.rs",
    "packages/d2b-core/src/bundle_resolver.rs",
    "packages/d2b-core/src/test_support.rs",
    "packages/d2b-core/tests/bundle_resolver_runner_intent_parity.rs",
    "packages/d2b-priv-broker/src/ops/storage_contract.rs",
    "packages/d2b-priv-broker/src/ops/tap.rs",
    "packages/d2b-priv-broker/src/runtime.rs",
    "packages/d2b-priv-broker/tests/w15_install_migrate.rs",
    "packages/d2bd/Cargo.toml",
    "packages/d2bd/src/kernel_module_check.rs",
    "packages/d2bd/src/lib.rs",
    "packages/d2bd/src/net_vm_bundle_gate.rs",
    "packages/d2bd/src/observability_export.rs",
    "packages/d2bd/src/provider_effects.rs",
    "packages/d2bd/src/provider_registry.rs",
    "packages/d2bd/src/storage_lifecycle.rs",
    "packages/d2bd/src/supervisor/stop_dag.rs",
    "packages/d2bd/tests/bundle_tampered_envelope.rs",
    "packages/d2bd/tests/common/mod.rs",
    "packages/d2bd/tests/public_status_socket.rs",
    "packages/xtask/src/main.rs",
    "packages/xtask/tests/policy_workspace.rs",
    "tests/golden/pinned/d2bd-startup-smoke.txt",
    "tests/unit/nix/cases/realm-workloads.nix",
    "tests/unit/nix/pinned/common.txt",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives under packages/xtask")
        .to_path_buf()
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn git_tracked_files() -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .expect("run git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8(entry.to_vec()).expect("tracked paths are UTF-8"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn workspace_metadata() -> serde_json::Value {
    let output = Command::new("cargo")
        .current_dir(repo_root().join("packages"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--all-features",
        ])
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse cargo metadata")
}

fn transitive_package_names(metadata: &serde_json::Value, root_package: &str) -> BTreeSet<String> {
    let package_names = metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .map(|package| {
            (
                package["id"].as_str().expect("package id").to_owned(),
                package["name"].as_str().expect("package name").to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let dependencies = metadata["resolve"]["nodes"]
        .as_array()
        .expect("metadata resolve nodes")
        .iter()
        .map(|node| {
            let ids = node["deps"]
                .as_array()
                .expect("node dependencies")
                .iter()
                .map(|dependency| {
                    dependency["pkg"]
                        .as_str()
                        .expect("dependency package id")
                        .to_owned()
                })
                .collect::<Vec<_>>();
            (
                node["id"].as_str().expect("resolve node id").to_owned(),
                ids,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let root_id = package_names
        .iter()
        .find_map(|(id, name)| (name == root_package).then(|| id.clone()))
        .unwrap_or_else(|| panic!("workspace package {root_package} not found"));

    let mut pending = VecDeque::from([root_id]);
    let mut visited = BTreeSet::new();
    let mut names = BTreeSet::new();
    while let Some(id) = pending.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }

        if let Some(name) = package_names.get(&id) {
            names.insert(name.clone());
        }
        if let Some(dependency_ids) = dependencies.get(&id) {
            pending.extend(dependency_ids.iter().cloned());
        }
    }
    names
}

fn declared_dependencies<'a>(
    metadata: &'a serde_json::Value,
    package_name: &str,
) -> &'a [serde_json::Value] {
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .find(|package| package["name"].as_str() == Some(package_name))
        .and_then(|package| package["dependencies"].as_array())
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("workspace package {package_name} dependencies not found"))
}

fn declared_features<'a>(
    metadata: &'a serde_json::Value,
    package_name: &str,
) -> &'a serde_json::Map<String, serde_json::Value> {
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .find(|package| package["name"].as_str() == Some(package_name))
        .and_then(|package| package["features"].as_object())
        .unwrap_or_else(|| panic!("workspace package {package_name} features not found"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum ShippedRustTargetKind {
    Binary,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum ShippedRustBuildKind {
    DeliveryWorkspace,
    GuestShellStatic,
    GuestStatic,
    Workspace,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShippedRustFlakePackage {
    output: String,
    build_kind: ShippedRustBuildKind,
    binary: String,
    main_program: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShippedRustPackage {
    cargo_package: String,
    target_kind: ShippedRustTargetKind,
    flake_package: Option<ShippedRustFlakePackage>,
}

fn git_file_flake_ref(root: &Path) -> String {
    assert!(root.is_absolute(), "flake repository root must be absolute");
    let mut encoded = String::from("git+file://");
    for byte in root.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn evaluate_flake_json(attribute: &str, apply: Option<&str>) -> Vec<u8> {
    let root = repo_root()
        .canonicalize()
        .expect("canonicalize flake repository root");
    let target = format!("{}#{attribute}", git_file_flake_ref(&root));
    let mut command = Command::new("nix");
    command
        .current_dir(&root)
        .args(["eval", "--json", "--impure", "--no-warn-dirty"])
        .arg(target);
    if let Some(apply) = apply {
        command.args(["--apply", apply]);
    }
    if std::env::var_os("NIX_CONFIG").is_none() {
        command.env("NIX_CONFIG", "experimental-features = nix-command flakes");
    }
    let output = command.output().expect("evaluate flake JSON");
    assert!(
        output.status.success(),
        "nix eval {attribute} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn evaluated_shipped_rust_packages() -> &'static [ShippedRustPackage] {
    static SHIPPED: OnceLock<Vec<ShippedRustPackage>> = OnceLock::new();
    SHIPPED.get_or_init(|| {
        serde_json::from_slice(&evaluate_flake_json("lib.shippedRustPackages", None))
            .expect("decode evaluated lib.shippedRustPackages JSON")
    })
}

fn evaluated_supported_systems() -> &'static [String] {
    static SYSTEMS: OnceLock<Vec<String>> = OnceLock::new();
    SYSTEMS.get_or_init(|| {
        serde_json::from_slice(&evaluate_flake_json("lib.supportedSystems", None))
            .expect("decode evaluated lib.supportedSystems JSON")
    })
}

fn evaluated_flake_package_outputs() -> &'static BTreeMap<String, BTreeSet<String>> {
    static OUTPUTS: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    OUTPUTS.get_or_init(|| {
        evaluated_supported_systems()
            .iter()
            .map(|system| {
                let outputs = serde_json::from_slice(&evaluate_flake_json(
                    &format!("packages.{system}"),
                    Some("builtins.attrNames"),
                ))
                .unwrap_or_else(|_| panic!("decode evaluated {system} flake package outputs"));
                (system.clone(), outputs)
            })
            .collect()
    })
}

fn metadata_package<'a>(
    metadata: &'a serde_json::Value,
    package_name: &str,
) -> &'a serde_json::Value {
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .expect("metadata workspace members")
        .iter()
        .map(|member| member.as_str().expect("workspace member id"))
        .collect::<BTreeSet<_>>();
    metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .find(|package| {
            package["name"].as_str() == Some(package_name)
                && package["id"]
                    .as_str()
                    .is_some_and(|id| workspace_members.contains(id))
        })
        .unwrap_or_else(|| {
            panic!("shipped Rust package {package_name} is absent from Cargo metadata")
        })
}

fn target_has_kind(target: &serde_json::Value, kind: &str) -> bool {
    target["kind"]
        .as_array()
        .expect("target kind")
        .iter()
        .any(|value| value.as_str() == Some(kind))
}

fn package_has_target_kind(package: &serde_json::Value, kind: &str) -> bool {
    package["targets"]
        .as_array()
        .expect("package targets")
        .iter()
        .any(|target| target_has_kind(target, kind))
}

fn shipped_production_rust_packages(metadata: &serde_json::Value) -> BTreeSet<String> {
    let shipped = evaluated_shipped_rust_packages();
    let mut declared = BTreeSet::new();
    let mut flake_outputs = BTreeSet::new();
    for entry in shipped {
        assert!(
            declared.insert(entry.cargo_package.clone()),
            "duplicate shipped Rust Cargo package {}",
            entry.cargo_package
        );
        let package = metadata_package(metadata, &entry.cargo_package);
        let target_kind = match entry.target_kind {
            ShippedRustTargetKind::Binary => "bin",
            ShippedRustTargetKind::Library => "lib",
        };
        assert!(
            package_has_target_kind(package, target_kind),
            "shipped Rust package {} has no {target_kind} target",
            entry.cargo_package
        );
        if let Some(flake_package) = &entry.flake_package {
            assert_eq!(
                entry.target_kind,
                ShippedRustTargetKind::Binary,
                "flake package {} must map to a binary Cargo package",
                flake_package.output
            );
            assert!(
                flake_outputs.insert(flake_package.output.clone()),
                "duplicate shipped Rust flake output {}",
                flake_package.output
            );
            assert!(
                !flake_package.output.is_empty() && !flake_package.binary.is_empty(),
                "shipped Rust flake mappings must use non-empty names"
            );
            assert!(
                package["targets"]
                    .as_array()
                    .expect("package targets")
                    .iter()
                    .any(|target| {
                        target["name"].as_str() == Some(&flake_package.binary)
                            && target_has_kind(target, "bin")
                    }),
                "flake output {} maps to missing binary {} in Cargo package {}",
                flake_package.output,
                flake_package.binary,
                entry.cargo_package
            );
            if let Some(main_program) = &flake_package.main_program {
                assert_eq!(
                    main_program, &flake_package.binary,
                    "flake output {} main program must match its selected binary",
                    flake_package.output
                );
            }
            match flake_package.build_kind {
                ShippedRustBuildKind::DeliveryWorkspace
                | ShippedRustBuildKind::GuestShellStatic
                | ShippedRustBuildKind::GuestStatic
                | ShippedRustBuildKind::Workspace => {}
            }
        }
    }

    let workspace_members = metadata["workspace_members"]
        .as_array()
        .expect("metadata workspace members")
        .iter()
        .map(|member| member.as_str().expect("workspace member id"))
        .collect::<BTreeSet<_>>();
    let mut cargo_roots = metadata["packages"]
        .as_array()
        .expect("metadata packages")
        .iter()
        .filter(|package| {
            package["id"]
                .as_str()
                .is_some_and(|id| workspace_members.contains(id))
        })
        .filter(|package| package_has_target_kind(package, "bin"))
        .map(|package| package["name"].as_str().expect("package name").to_owned())
        .filter(|package| !NON_PRODUCTION_BINARY_PACKAGES.contains(&package.as_str()))
        .collect::<BTreeSet<_>>();
    cargo_roots.extend(
        shipped
            .iter()
            .filter(|entry| entry.target_kind == ShippedRustTargetKind::Library)
            .map(|entry| entry.cargo_package.clone()),
    );
    assert_eq!(
        declared, cargo_roots,
        "evaluated lib.shippedRustPackages must exactly match Cargo production roots"
    );
    declared
}

#[test]
fn workspace_names_contract_crate_by_role() {
    let workspace = read_repo_file("packages/Cargo.toml");
    assert!(
        workspace.contains(&format!("\"{CONTRACTS_CRATE}\"")),
        "main workspace must include the contract/DTO crate by role"
    );
    assert!(
        !workspace.contains(&format!("\"{}{}\"", "d2b", "-ipc")),
        "main workspace must not reintroduce the old transport-shaped contract crate name"
    );

    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    assert!(
        manifest.contains(&format!("name = \"{CONTRACTS_CRATE}\"")),
        "contract crate manifest must use the role-based package name"
    );
}

#[test]
fn implementation_crates_are_base_first_and_workspace_members_are_sorted() {
    let workspace = read_repo_file("packages/Cargo.toml");
    for package in IMPLEMENTATION_CRATES {
        assert!(
            workspace.contains(&format!("\"{package}\"")),
            "workspace must contain base-first implementation crate {package}"
        );
    }

    for forbidden in ["d2b-host-providers", "d2b-unix-session"] {
        assert!(
            !workspace.contains(forbidden),
            "workspace must not contain implementation-before-base crate {forbidden}"
        );
    }

    let members = workspace
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(members, _)| members)
        .expect("workspace members array");
    let actual = members
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.strip_suffix("\","))
        .collect::<Vec<_>>();
    let mut sorted = actual.clone();
    sorted.sort_unstable();
    assert_eq!(
        actual, sorted,
        "workspace members must remain alphanumerically sorted"
    );
}

#[test]
fn w4_provider_workspace_inventory_is_reserved_and_dependency_minimal() {
    let metadata = workspace_metadata();
    let workspace = read_repo_file("packages/Cargo.toml");
    let members = workspace
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(members, _)| members)
        .expect("workspace members array")
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.strip_suffix("\","))
        .collect::<Vec<_>>();
    let mut sorted_members = members.clone();
    sorted_members.sort_unstable();
    assert_eq!(
        members, sorted_members,
        "workspace members must remain alphanumerically sorted"
    );

    for package in W4_PROVIDER_CRATES
        .iter()
        .copied()
        .chain(std::iter::once(W4_SUPPORT_CRATE))
    {
        assert!(
            members.contains(&package),
            "workspace members must include {package}"
        );
        assert!(
            workspace.contains(&format!(
                "{package} = {{ path = \"{package}\", version = \"2.0.0\", default-features = false }}"
            )),
            "workspace dependencies must own default-empty {package}"
        );

        let manifest = read_repo_file(&format!("packages/{package}/Cargo.toml"));
        assert!(
            manifest.contains(&format!("name = \"{package}\"")),
            "{package} manifest has the wrong package name"
        );
        for required in [
            "version.workspace = true",
            "edition.workspace = true",
            "publish = false",
            "[features]\ndefault = []",
        ] {
            assert!(
                manifest.contains(required),
                "{package} manifest is missing {required:?}"
            );
        }
        let source = read_repo_file(&format!("packages/{package}/src/lib.rs"));
        assert!(
            source.starts_with("//!") && source.contains("#![forbid(unsafe_code)]"),
            "{package} must document its purpose and forbid unsafe code"
        );
    }

    for package in W4_PROVIDER_CRATES.iter().copied() {
        let manifest = read_repo_file(&format!("packages/{package}/Cargo.toml"));
        let contracts_dependency = "d2b-contracts = { workspace = true, default-features = false, features = [\"v2-provider\"] }";
        assert!(
            manifest.contains(contracts_dependency),
            "{package} must depend on the canonical provider contracts: {contracts_dependency}"
        );
        if !W4_UNAVAILABLE_PROVIDER_SCAFFOLDS.contains(&package) {
            let provider_dependency =
                "d2b-provider = { workspace = true, default-features = false }";
            assert!(
                manifest.contains(provider_dependency),
                "{package} must depend on the canonical live-provider boundary: {provider_dependency}"
            );
        }
        let dependency_names = transitive_package_names(&metadata, package);
        for forbidden in ["d2bd", "d2b-priv-broker"] {
            assert!(
                !dependency_names.contains(forbidden),
                "{package} must not transitively depend on {forbidden}"
            );
        }
    }

    assert!(
        !transitive_package_names(&metadata, "d2b-provider-runtime-local").contains("d2b-host"),
        "the local runtime provider must not transitively depend on d2b-host"
    );
    let local_runtime_dependencies = declared_dependencies(&metadata, "d2b-provider-runtime-local")
        .iter()
        .map(|dependency| dependency["name"].as_str().expect("dependency name"))
        .collect::<BTreeSet<_>>();
    assert!(
        !local_runtime_dependencies.contains("serde_json"),
        "the local runtime provider must not directly depend on serde_json"
    );

    for package in
        std::iter::once(W4_SUPPORT_CRATE).chain(W4_UNAVAILABLE_PROVIDER_SCAFFOLDS.iter().copied())
    {
        let package_features = declared_features(&metadata, package);
        assert_eq!(
            package_features
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["default"],
            "{package} must not add feature-forwarding escape hatches"
        );
        assert_eq!(
            package_features["default"].as_array().map(Vec::as_slice),
            Some([].as_slice()),
            "{package} default feature set must remain empty"
        );

        let allowed_direct = if package == W4_SUPPORT_CRATE {
            ["async-trait", "serde", "tokio"].as_slice()
        } else {
            [
                "d2b-azure-vm-fake-sdk",
                "d2b-contracts",
                "d2b-provider-toolkit",
                "tokio",
            ]
            .as_slice()
        };
        for dependency in declared_dependencies(&metadata, package) {
            let dependency_name = dependency["name"].as_str().expect("dependency name");
            assert!(
                allowed_direct.contains(&dependency_name),
                "{package} has unapproved direct dependency {dependency_name}"
            );
            assert!(
                dependency["optional"].as_bool() == Some(false),
                "{package} must not hide dependencies behind optional features"
            );
            let features = dependency["features"]
                .as_array()
                .expect("dependency features")
                .iter()
                .map(|feature| feature.as_str().expect("feature name"))
                .collect::<BTreeSet<_>>();
            let allowed_features = match dependency_name {
                "d2b-contracts" => BTreeSet::from(["v2-provider"]),
                "serde" => BTreeSet::from(["derive"]),
                "tokio" => BTreeSet::from([
                    "macros",
                    "rt",
                    "rt-multi-thread",
                    "sync",
                    "test-util",
                    "time",
                ]),
                _ => BTreeSet::new(),
            };
            assert!(
                features.is_subset(&allowed_features),
                "{package} enables unapproved {dependency_name} features: {features:?}"
            );
        }

        let dependencies = transitive_package_names(&metadata, package);
        for dependency in dependencies {
            let forbidden = dependency.starts_with("azure")
                || matches!(
                    dependency.as_str(),
                    "curl"
                        | "hyper"
                        | "hyper-util"
                        | "openssl"
                        | "reqwest"
                        | "rustls"
                        | "tokio-tungstenite"
                        | "tungstenite"
                        | "ureq"
                );
            assert!(
                !forbidden,
                "{package} must not acquire live Azure/network dependency {dependency}"
            );
        }
    }

    for production_package in shipped_production_rust_packages(&metadata) {
        let dependencies = transitive_package_names(&metadata, &production_package);
        for forbidden in W4_UNAVAILABLE_PROVIDER_SCAFFOLDS.iter().copied() {
            assert!(
                !dependencies.contains(forbidden),
                "{production_package} must not include unavailable Azure VM provider {forbidden}"
            );
        }
    }
}

#[test]
fn evaluated_shipped_rust_packages_match_cargo_production_roots() {
    let metadata = workspace_metadata();
    let shipped = shipped_production_rust_packages(&metadata);
    for required in REQUIRED_SHIPPED_PRODUCTION_PACKAGES {
        assert!(
            shipped.contains(*required),
            "shipped production package inventory is missing {required}"
        );
    }

    let mut expected_outputs = NON_RUST_FLAKE_PACKAGE_OUTPUTS
        .iter()
        .map(|output| (*output).to_owned())
        .collect::<BTreeSet<_>>();
    expected_outputs.extend(
        evaluated_shipped_rust_packages()
            .iter()
            .filter_map(|entry| entry.flake_package.as_ref())
            .map(|mapping| mapping.output.clone()),
    );
    let outputs_by_system = evaluated_flake_package_outputs();
    assert_eq!(
        outputs_by_system.keys().collect::<BTreeSet<_>>(),
        evaluated_supported_systems()
            .iter()
            .collect::<BTreeSet<_>>(),
        "every supported flake system must be evaluated"
    );
    for (system, outputs) in outputs_by_system {
        assert_eq!(
            outputs, &expected_outputs,
            "every {system} flake package output must have an exact shipped-Rust or non-Rust classification"
        );
    }
}

#[test]
fn unavailable_azure_vm_scaffold_descriptors_have_no_live_provider_surface() {
    let metadata = workspace_metadata();
    let tracked_files = git_tracked_files();

    for package in W4_UNAVAILABLE_PROVIDER_SCAFFOLDS.iter().copied() {
        let dependencies = declared_dependencies(&metadata, package);
        assert!(
            !dependencies
                .iter()
                .any(|dependency| dependency["name"].as_str() == Some("d2b-provider")),
            "{package} must not depend on the live provider implementation boundary"
        );

        let source_prefix = format!("packages/{package}/src/");
        let source_paths = tracked_files
            .iter()
            .filter(|path| path.starts_with(&source_prefix) && path.ends_with(".rs"))
            .collect::<Vec<_>>();
        assert!(
            !source_paths.is_empty(),
            "{package} must have tracked Rust sources"
        );
        for path in source_paths {
            let source = read_repo_file(path);
            for forbidden in ["ProviderFactory", "ProviderInstance"] {
                assert!(
                    !source.contains(forbidden),
                    "{package} unavailable scaffold must not expose {forbidden} in {path}"
                );
            }
        }

        let source = read_repo_file(&format!("packages/{package}/src/lib.rs"));
        assert!(
            source.contains(
                "pub const fn advertised_capabilities(&self) -> &'static [ProviderMethod] {\n        &[]\n    }"
            ) && source.contains(
                "pub const fn is_registerable(&self) -> bool {\n        false\n    }"
            ),
            "{package} descriptor must remain capability-empty and non-registerable"
        );
    }
}

#[test]
fn daemon_scm_rights_receive_is_atomic_bounded_and_raii_owned() {
    let daemon = read_repo_file("packages/d2bd/src/lib.rs");
    for required in [
        "const LINUX_SCM_MAX_FD: usize = 253;",
        "rustix::cmsg_space!(ScmRights(fd_capacity), ScmCredentials(1))",
        "rustix::net::RecvFlags::CMSG_CLOEXEC",
        "rustix::net::RecvAncillaryMessage::ScmRights",
        "message.flags.bits() & MSG_CTRUNC_FLAG != 0",
        ".map(IntoRawFd::into_raw_fd)",
        "scm_rights_receive_accepts_more_than_eight_and_linux_maximum",
        "scm_rights_truncation_fails_closed_without_descriptor_growth",
    ] {
        assert!(
            daemon.contains(required),
            "daemon SCM_RIGHTS receive policy is missing {required}"
        );
    }
    for forbidden in ["cmsg_space!([RawFd; 8])", "recvmsg::<UnixAddr>"] {
        assert!(
            !daemon.contains(forbidden),
            "daemon SCM_RIGHTS receive policy still contains {forbidden}"
        );
    }
}

#[test]
fn daemon_provider_composition_is_exact_startup_owned_and_credential_free() {
    let metadata = workspace_metadata();
    let dependencies = declared_dependencies(&metadata, "d2bd")
        .iter()
        .map(|dependency| {
            dependency["name"]
                .as_str()
                .expect("dependency name")
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    for required in [
        "d2b-provider",
        "d2b-provider-audio-pipewire-vhost-user",
        "d2b-provider-device-host-mediated",
        "d2b-provider-display-wayland",
        "d2b-provider-network-local-realm",
        "d2b-provider-observability-local",
        "d2b-provider-runtime-azure-container-apps",
        "d2b-provider-runtime-local",
        "d2b-provider-storage-local",
        "d2b-provider-substrate-host",
        "d2b-provider-transport-azure-relay",
        "d2b-provider-transport-local",
    ] {
        assert!(
            dependencies.contains(required),
            "d2bd is missing first-party provider dependency {required}"
        );
    }
    for forbidden in [
        "d2b-priv-broker",
        "d2b-provider-credential-entra",
        "d2b-provider-credential-managed-identity",
        "d2b-provider-credential-secret-service",
        "d2b-provider-infrastructure-azure-vm",
        "d2b-provider-runtime-azure-vm",
    ] {
        assert!(
            !dependencies.contains(forbidden),
            "d2bd must not own credential or unavailable provider {forbidden}"
        );
    }
    let transitive_dependencies = transitive_package_names(&metadata, "d2bd");
    for forbidden in [
        "d2b-priv-broker",
        "d2b-provider-credential-entra",
        "d2b-provider-credential-managed-identity",
        "d2b-provider-credential-secret-service",
        "d2b-provider-infrastructure-azure-vm",
        "d2b-provider-runtime-azure-vm",
    ] {
        assert!(
            !transitive_dependencies.contains(forbidden),
            "d2bd must not transitively acquire credential or unavailable provider {forbidden}"
        );
    }

    let daemon = read_repo_file("packages/d2bd/src/lib.rs");
    assert!(
        daemon.contains(
            "provider_registry: Arc<OnceLock<provider_registry::StartupProviderRegistry>>"
        ) && daemon.contains("provider_registry: Arc::new(OnceLock::new())")
            && daemon.contains("async fn activate_provider_registry")
            && daemon.contains("provider_registry::compose_startup_registry_with_policy(")
            && daemon.contains("provider_registry::probe_startup_registry")
            && daemon.contains("fn provider_registry("),
        "the daemon must compose, probe, retain, and expose one startup-owned provider registry"
    );
    let state_construction = daemon
        .find("let state = Arc::new(ServerState")
        .expect("production ServerState construction");
    let registry_activation = daemon
        .find("activate_provider_registry(&state, test_bundle_policy.as_ref()).await?;")
        .expect("production provider registry activation");
    assert!(
        state_construction < registry_activation
            && !daemon.contains("#[allow(dead_code)]\n    provider_registry"),
        "the provider registry must initialize once after ServerState and remain a live field"
    );

    let composition = read_repo_file("packages/d2bd/src/provider_registry.rs");
    for required in [
        "ProviderRegistryBuilder::new",
        "register_factory",
        "register_constructed",
        "provider_capabilities_are_dispatchable",
        "LocalRuntimeProviderFactory",
        "LocalTransportFactory",
        "HostSubstrateProviderFactory",
        "WaylandDisplayFactory",
        "LocalRealmNetworkFactory",
        "LocalStorageFactory",
        "HostMediatedDeviceFactory",
        "PipewireVhostUserAudioFactory",
        "LocalObservabilityFactory",
        "AzureContainerAppsRuntimeProviderFactory",
        "AzureRelayProviderFactory",
        "AzureVmForbidden",
        "AgentProviderBinding",
    ] {
        assert!(
            composition.contains(required),
            "provider composition is missing exact integration surface {required}"
        );
    }
    for binding in [
        "LocalRuntime",
        "LocalTransport",
        "HostSubstrate",
        "WaylandDisplay",
        "LocalRealmNetwork",
        "LocalStorage",
        "HostMediatedDevice",
        "PipewireVhostUserAudio",
        "LocalObservability",
    ] {
        assert!(
            composition.contains(&format!("Self::{binding}")),
            "host provider composition is missing exact binding {binding}"
        );
    }
    for constructor in [
        "LocalRuntimeProviderFactoryEntry::new(",
        "LocalTransportFactory::new(",
        "HostSubstrateFactoryEntry::new(",
        "WaylandDisplayFactory::new(",
        "LocalRealmNetworkFactory::new(",
        "LocalStorageFactory::new(",
        "HostMediatedDeviceFactoryEntry::new(",
        "PipewireVhostUserAudioFactoryEntry::new(",
        "LocalObservabilityFactoryEntry::new(",
    ] {
        assert!(
            composition.contains(constructor),
            "host provider composition does not invoke exact constructor {constructor}"
        );
    }
    assert!(
        composition.contains("key != factory_key(descriptor)")
            && composition.contains("instance.descriptor() != *descriptor")
            && composition.contains("instance.capabilities() != descriptor.capabilities"),
        "constructed instances must match the exact descriptor, factory, and capabilities"
    );
    assert!(
        composition.contains("ProviderType::Infrastructure | ProviderType::Credential => false")
            && composition.contains("SYSTEMD_USER_IMPLEMENTATION_ID | ACA_IMPLEMENTATION_ID")
            && composition.contains("(ProviderType::Transport, AZURE_RELAY_IMPLEMENTATION_ID)"),
        "host composition must exclude credentials and agent-only implementations"
    );
    for required in [
        "bundle.bundle_version != PROVIDER_BUNDLE_VERSION",
        "bundle.schema_version != PROVIDER_BUNDLE_SCHEMA_VERSION",
        "bundle.bundle_hash.is_none()",
        "!artifact_hashes.contains_key(provider_path)",
        "runner.source != ResolvedRunnerSource::ExplicitProcessNode",
        "identity.realm_path.target_form()",
        "WorkloadId::derive(&expected_realm_id, &workload_name)",
        "DuplicateRuntimeMapping",
    ] {
        assert!(
            composition.contains(required),
            "provider activation is missing fail-closed mapping check {required}"
        );
    }
    for required in [
        "dispatch_provider_or_broker_vm_start",
        "dispatch_provider_or_broker_vm_stop_as",
        "dispatch_provider_or_broker_vm_restart_as",
        "provider_registry::invoke_runtime_lifecycle(",
    ] {
        assert!(
            daemon.contains(required),
            "production lifecycle routing is missing provider admission path {required}"
        );
    }
    for required in [
        "provider_lifecycle_deadline(",
        "registry\n        .admit(",
        "AdmissionOptions {",
        "deadline_after: deadline.duration",
        "ProviderFailureKind::AmbiguousMutation",
        "RetryClass::AfterObservation",
    ] {
        assert!(
            composition.contains(required),
            "mapped runtime lifecycle is missing bounded admission or ambiguous-timeout handling {required}"
        );
    }
    for required in [
        "validate_runtime_lifecycle_budgets",
        "LifecycleBudgetExceeded",
        "mapped_runtime_lifecycle_budgets",
        "MAX_PROVIDER_REQUEST_LIFETIME_MS",
    ] {
        assert!(
            composition.contains(required),
            "provider startup must reject underbudgeted lifecycle mapping without truncation {required}"
        );
    }
    assert!(
        !composition.contains(".clamp("),
        "provider lifecycle deadlines must never silently truncate daemon cleanup budgets"
    );
    for required in [
        "mapped_runtime_start_budget_for_dag",
        "mapped_runtime_stop_budget_for_roles",
        "mapped_runtime_role_cleanup_budget",
        "mapped_runtime_graceful_shutdown_budget",
        "let request = ch_api::DEFAULT_TIMEOUT",
        "let trailing_poll = ch_api::DEFAULT_TIMEOUT",
        "checked_lifecycle_budget_add(stop, start)",
        "CGROUP_KILL_BROKER_TIMEOUT",
        "CGROUP_EMPTY_POST_KILL_WAIT",
        "configured_usbip_claim_count",
        "mapped_runtime_usbip_lifecycle_budgets",
        "USBIP_STRICT_RECONCILE_TIMEOUT",
        "GUEST_CONTROL_USBIP_IMPORT_TIMEOUT",
        "USBIP_STOP_FIREWALL_WITHDRAWAL_BOUND",
        "USBIP_STOP_HOST_UNBIND_BOUND",
        "USBIP_STOP_PROXY_RECONCILE_BOUND",
        "match dispatch_broker_request_with_timeout(state, request, timeout)",
        "ensure_runtime_restart_budget",
        "wait_for_mapped_lifecycle(&lock_class)",
    ] {
        assert!(
            daemon.contains(required),
            "mapped runtime routing is missing full budget or retained serialization surface {required}"
        );
    }

    let effects = read_repo_file("packages/d2bd/src/provider_effects.rs");
    for required in [
        "dispatch_broker_vm_start_on_blocking_adapter",
        "dispatch_broker_vm_stop_on_blocking_adapter",
        "resolve_current_runtime_route",
        "still_alive_same_start_time",
        "ProviderLifecycleTasks",
        "LifecycleMutationKey::from_request",
        "std::thread::Builder::new()",
        "task.wait().await",
        "begin_mapped_lifecycle",
        "let _lifecycle_permit = lifecycle_permit",
    ] {
        assert!(
            effects.contains(required),
            "live runtime adapter must invoke the existing daemon authority seam {required}"
        );
    }
    let concurrency = read_repo_file("packages/d2bd/src/concurrency.rs");
    for required in [
        "pub struct MappedLifecyclePermit",
        "pub fn wait_for_mapped_lifecycle",
        "pub fn begin_mapped_lifecycle",
        "impl Drop for MappedLifecyclePermit",
    ] {
        assert!(
            concurrency.contains(required),
            "detached lifecycle workers must retain same-VM serialization authority {required}"
        );
    }
    assert!(
        !effects.contains("dispatch_broker_vm_start(&")
            && !effects.contains("dispatch_broker_vm_stop_as(&"),
        "async provider ports must not re-enter synchronous Tokio lifecycle bridges"
    );
    let export_store = read_repo_file("packages/d2bd/src/observability_export.rs");
    for required in [
        "create_new(true)",
        ".mode(0o600)",
        "file.sync_all()",
        "fs::rename(&temp_path, &artifact_path)",
        "directory.sync_all()",
        "pub(crate) fn inspect(",
    ] {
        assert!(
            export_store.contains(required),
            "durable observability export store is missing {required}"
        );
    }
    assert!(
        !export_store.contains("tracing::") && !export_store.contains(".display()"),
        "observability export persistence must not log or format raw host paths"
    );
    let observability = read_repo_file("packages/d2b-provider-observability-local/src/lib.rs");
    for required in [
        "encode_json_lines",
        "encode_otlp_export",
        "encoded_payload",
        "encoded_bytes > self.bounds.max_bytes",
    ] {
        assert!(
            observability.contains(required),
            "provider-owned bounded export sink is missing {required}"
        );
    }
    for required in [
        "tokio::task::spawn_blocking",
        "store.persist(",
        "inspection.encoded_bytes != encoded_bytes",
    ] {
        assert!(
            effects.contains(required),
            "daemon observability export adapter is missing {required}"
        );
    }
    let provider_contract = read_repo_file("packages/d2b-contracts/src/provider_registry_v2.rs");
    assert!(
        provider_contract.contains("pub struct ProviderRegistryV2")
            && provider_contract.contains("pub enum ProviderBindingV2")
            && provider_contract.contains("LocalRuntime(LocalRuntimeProviderBindingV2)")
            && provider_contract.contains("deny_unknown_fields"),
        "the private generated provider artifact must use a closed canonical DTO"
    );
    let provider_emitter = read_repo_file("nixos-modules/provider-registry-v2-json.nix");
    assert!(
        provider_emitter.contains("cfg._index.realms.workloads.enabled")
            && provider_emitter.contains("provider-registry-v2.json")
            && provider_emitter.contains("contractPrivateNonSecret")
            && !provider_emitter.contains("runtime.execute"),
        "the provider registry artifact must derive from explicit canonical workloads only"
    );
    let instance = read_repo_file("packages/d2b-provider/src/instance.rs");
    assert!(
        instance.contains("ProviderMethod::RuntimeExecute") && instance.contains("!matches!"),
        "RuntimeExecute must remain excluded from live provider dispatch"
    );

    let manifest = read_repo_file("packages/d2bd/Cargo.toml");
    assert!(
        manifest.contains("d2b-provider-aca = { workspace = true }")
            && manifest.contains("d2b-provider-relay = { workspace = true }")
            && manifest.contains("Retained until the v1 ACA and Relay"),
        "legacy ACA and Relay dependencies need an explicit v1 parity boundary"
    );
}

#[test]
fn standalone_proofs_isolate_mixed_toolchain_targets() {
    let script = read_repo_file("tests/test-proofs.sh");
    assert!(
        script.contains("CARGO_TARGET_DIR/d2b-proofs/$RUSTUP_TOOLCHAIN")
            && script.contains("clippy_target_args=(--target-dir")
            && script.contains("test_target_args=(--target-dir")
            && script.contains("d2b_activate_rust_toolchain_path \"$pinned_channel\"")
            && script.contains("CLIPPY_DRIVER=\"$proof_clippy_driver\""),
        "standalone proof crates must not share target metadata across rustc versions"
    );
    let rust_gate = read_repo_file("tests/test-rust.sh");
    assert!(
        rust_gate.contains(
            "export RUSTC=\"$gate_rustc\" RUSTDOC=\"$gate_rustdoc\" CLIPPY_DRIVER=\"$gate_clippy_driver\""
        )
            && rust_gate.contains("RUSTC does not match packages/rust-toolchain.toml"),
        "workspace Rust gate must pin the compiler and clippy executables used by Cargo"
    );
    let delivery_tools = read_repo_file("pkgs/delivery-tools.nix");
    assert!(
        delivery_tools.contains("rust-bin.stable.${rustStableVersion}.default"),
        "delivery shell must provide matching clippy/rustfmt with pinned stable Rust"
    );
}

#[test]
fn post_wave_cleanup_removes_branches_worktree_targets_and_gc_roots() {
    let agents = read_repo_file("AGENTS.md");
    for required in [
        "Delete the merged remote feature branch.",
        "Remove the finished local worktree.",
        "Delete the corresponding local feature branch.",
        "Run `nix-collect-garbage`",
        "verify `git worktree list` contains only",
    ] {
        assert!(
            agents.contains(required),
            "post-wave cleanup contract is missing: {required}"
        );
    }
}

#[test]
fn adr_delivery_rules_require_dependency_ready_parallelism() {
    let agents = read_repo_file("AGENTS.md");
    for required in [
        "This is a positive launch requirement, not merely permission.",
        "launch every newly ready speculative wave in the same coordination cycle",
        "Anti-serialization invariant",
        "Build a file-overlap graph for all ready scopes.",
        "distinct components MUST run concurrently",
        "A persistent agent owns one coherent component.",
        "Do not repeatedly expand one",
        "A launch count below the ready count",
        "Resource limits constrain heavy validation, not implementation parallelism.",
    ] {
        assert!(
            agents.contains(required),
            "AGENTS.md anti-serialization contract is missing: {required}"
        );
    }

    let adr = read_repo_file("docs/adr/0045-provider-and-transport-framework.md");
    for required in [
        "Create `adr0045-post-w4-contracts` from `W4-F`.",
        "before W6/W7 create final snapshots or run final panels",
        "`adr0045-w8-integration` as a Git Town child of the linearized W7 head",
        "Serial ownership stops at the smallest connected component",
        "the gate child duplicates that same locked open-file description",
    ] {
        assert!(
            adr.contains(required),
            "ADR 0045 post-W4 parallel execution contract is missing: {required}"
        );
    }
}

#[test]
fn stale_ipc_crate_name_is_absent_from_current_sources() {
    let old_hyphen = format!("{}{}", "d2b", "-ipc");
    let old_underscore = format!("{}{}", "d2b", "_ipc");
    let self_path = "packages/xtask/tests/policy_workspace.rs";
    let mut violations = Vec::new();
    for rel in git_tracked_files() {
        if rel == self_path {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(repo_root().join(&rel)) else {
            continue;
        };
        if content.contains(&old_hyphen) || content.contains(&old_underscore) {
            violations.push(rel);
        }
    }
    assert!(
        violations.is_empty(),
        "stale contract crate references remain:\n{}",
        violations.join("\n")
    );
}

#[test]
fn focused_packages_share_workspace_lock_and_keep_supply_chain_policy() {
    let root = repo_root();
    let main_workspace = read_repo_file("packages/Cargo.toml");
    let flake = read_repo_file("flake.nix");
    assert!(
        flake.contains("packages/Cargo.lock"),
        "flake supply-chain gates must use the canonical workspace lock"
    );
    for package in FOCUSED_POLICY_PACKAGES {
        assert!(
            main_workspace.contains(&format!("\"{package}\"")),
            "main workspace members must include {package}"
        );
        for required in ["Cargo.toml", "deny.toml"] {
            let path = root.join("packages").join(package).join(required);
            assert!(path.exists(), "{} must exist", path.display());
        }

        assert!(
            !root
                .join("packages")
                .join(package)
                .join("Cargo.lock")
                .exists(),
            "{package} must not carry a nested lockfile"
        );
        assert!(
            flake.contains(&format!("packages/{package}/deny.toml")),
            "flake supply-chain gates must cover {package}"
        );
    }
}

#[test]
fn v2_foundation_crates_are_default_empty_and_not_publishable() {
    let root = repo_root();
    let workspace = read_repo_file("packages/Cargo.toml");
    for package in V2_FOUNDATION_CRATES {
        assert!(
            workspace.contains(&format!("\"{package}\""))
                && workspace.contains(&format!(
                    "{package} = {{ path = \"{package}\", version = \"2.0.0\", default-features = false }}"
                )),
            "workspace must own {package} with default features disabled"
        );
        let manifest = read_repo_file(&format!("packages/{package}/Cargo.toml"));
        for required in [
            "version.workspace = true",
            "rust-version.workspace = true",
            "publish = false",
            "[features]\ndefault = []",
            "[lints]\nworkspace = true",
        ] {
            assert!(
                manifest.contains(required),
                "{package} manifest is missing {required:?}"
            );
        }

        assert!(
            !root
                .join("packages")
                .join(package)
                .join("Cargo.lock")
                .exists(),
            "{package} must use the workspace lockfile"
        );
        for dependency in ["d2b-contracts", "d2b-provider", "d2b-session", "ttrpc"] {
            if manifest.contains(&format!("{dependency} =")) {
                assert!(
                    manifest.lines().any(|line| {
                        line.starts_with(&format!("{dependency} ="))
                            && line.contains("default-features = false")
                    }),
                    "{package} must disable default features for {dependency}"
                );
            }
        }
    }
}

#[test]
fn v2_foundation_delivery_fingerprints_cover_every_tracked_file() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read_repo_file("delivery/manifest.json")).expect("delivery manifest");
    let actual = manifest["contract_fingerprints"]
        .as_array()
        .expect("contract fingerprints")
        .iter()
        .map(|row| row["path"].as_str().expect("fingerprint path").to_owned())
        .filter(|path| {
            V2_FOUNDATION_CRATES
                .iter()
                .any(|package| path.starts_with(&format!("packages/{package}/")))
                || path == "docs/reference/v2-foundation-crates.md"
                || path == "packages/xtask/tests/policy_workspace.rs"
        })
        .collect::<BTreeSet<_>>();

    let mut expected = git_tracked_files()
        .into_iter()
        .filter(|path| {
            V2_FOUNDATION_CRATES
                .iter()
                .any(|package| path.starts_with(&format!("packages/{package}/")))
        })
        .collect::<BTreeSet<_>>();
    expected.insert("docs/reference/v2-foundation-crates.md".to_owned());
    expected.insert("packages/xtask/tests/policy_workspace.rs".to_owned());

    assert_eq!(actual, expected);
}

#[test]
fn w4_provider_delivery_fingerprints_cover_every_reserved_file() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read_repo_file("delivery/manifest.json")).expect("delivery manifest");
    let actual = manifest["contract_fingerprints"]
        .as_array()
        .expect("contract fingerprints")
        .iter()
        .map(|row| row["path"].as_str().expect("fingerprint path").to_owned())
        .collect::<BTreeSet<_>>();

    let mut expected = git_tracked_files()
        .into_iter()
        .filter(|path| {
            W4_PROVIDER_CRATES
                .iter()
                .chain(std::iter::once(&W4_SUPPORT_CRATE))
                .any(|package| path.starts_with(&format!("packages/{package}/")))
        })
        .collect::<BTreeSet<_>>();
    expected.insert("docs/reference/v2-provider-implementations.md".to_owned());
    expected.extend(
        PROVIDER_INTEGRATION_FILES
            .iter()
            .map(|path| (*path).to_owned()),
    );

    let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "W4 provider delivery fingerprints are missing:\n{}",
        missing.join("\n")
    );
}

#[test]
fn shared_contract_policy_freezes_services_dependencies_and_ownership() {
    let root = repo_root();
    let policy = xtask::wave_policy::read_policy(&root).expect("shared-contract policy");
    let frozen = policy
        .frozen_service_packages
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        frozen,
        d2b_contracts::v2_services::SERVICE_PACKAGES
            .into_iter()
            .collect()
    );
    assert_eq!(
        policy
            .waves
            .iter()
            .map(|wave| (wave.wave.as_str(), wave.responsibility.as_str()))
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::from([
            ("w5", "runtime-service-and-dispatch-implementation"),
            ("w6", "user-desktop-device-service-implementation"),
            ("w7", "declarative-nix-process-and-resource-emission"),
        ])
    );

    let generated =
        read_repo_file("packages/d2b-contracts/src/generated_v2_services/broker_ttrpc.rs");
    for method in &policy.broker_typed_methods {
        let method_name = method.method.to_ascii_lowercase();
        assert!(
            generated.contains(&format!(
                "pub async fn {method_name}(&self, ctx: ttrpc::context::Context, req: &super::broker::{}) -> ::ttrpc::Result<super::broker::{}>",
                method.request, method.response
            )),
            "generated broker binding does not freeze {} as {} -> {}",
            method.method,
            method.request,
            method.response
        );
    }

    let workspace = read_repo_file("packages/Cargo.toml");
    for dependency in &policy.workspace_dependencies {
        let line = workspace
            .lines()
            .find(|line| line.starts_with(&format!("{} = ", dependency.name)))
            .unwrap_or_else(|| panic!("workspace dependency {} is absent", dependency.name));
        assert!(
            line.contains(&format!("\"{}\"", dependency.requirement)),
            "workspace dependency {} does not retain requirement {}",
            dependency.name,
            dependency.requirement
        );
    }
    let lock = read_repo_file("packages/Cargo.lock");
    for (name, version) in [("command-fds", "0.3.3"), ("oo7", "0.6.0")] {
        assert!(
            lock.contains(&format!("name = \"{name}\"\nversion = \"{version}\"")),
            "{name} {version} must be frozen in the workspace lock"
        );
    }

    for required in [
        "packages/Cargo.lock",
        "packages/Cargo.toml",
        "packages/d2b-contracts/src/v2_services.rs",
        "packages/d2b-realm-core/src/allocator.rs",
    ] {
        assert!(
            policy
                .protected_paths
                .binary_search(&required.to_owned())
                .is_ok(),
            "shared contract policy does not protect {required}"
        );
    }
}

#[test]
fn provider_registry_v2_has_one_canonical_artifact_family() {
    let actual = git_tracked_files()
        .into_iter()
        .filter(|path| {
            path.ends_with("/provider-registry-v2.json")
                || path.ends_with("/provider-registry-v2.md")
                || path.ends_with("/provider-registry-v2-json.nix")
                || path.ends_with("/provider_registry_v2.rs")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual,
        BTreeSet::from([
            "docs/reference/schemas/v2/provider-registry-v2.json".to_owned(),
            "docs/reference/schemas/v2/provider-registry-v2.md".to_owned(),
            "nixos-modules/provider-registry-v2-json.nix".to_owned(),
            "packages/d2b-contracts/src/provider_registry_v2.rs".to_owned(),
        ])
    );
}

#[test]
fn v2_foundation_io_surfaces_are_async_first() {
    let client = read_repo_file("packages/d2b-client/src/client.rs");
    let connector = read_repo_file("packages/d2b-client/src/session.rs");
    let session_driver = read_repo_file("packages/d2b-session/src/driver.rs");
    let provider_rpc = read_repo_file("packages/d2b-provider/src/rpc.rs");
    for required in [
        "pub async fn connect",
        "pub async fn invoke",
        "pub async fn invoke_with_attachments",
        "pub async fn named_stream",
    ] {
        assert!(client.contains(required), "client is missing {required}");
    }
    assert!(
        connector.contains("#[async_trait]")
            && connector.contains("pub trait ComponentSessionConnector")
            && connector.contains("async fn connect"),
        "client connector must be async"
    );
    assert!(
        session_driver.contains("#[async_trait]")
            && session_driver.contains("pub trait ComponentSessionDriver")
            && session_driver.contains("async fn start_ttrpc")
            && session_driver.contains("async fn receive_ttrpc")
            && session_driver.contains("async fn complete_ttrpc")
            && !session_driver.contains("async fn invoke"),
        "canonical session driver must be async"
    );
    assert!(
        provider_rpc.contains("#[async_trait]")
            && provider_rpc.contains("pub trait AuthenticatedProviderRpc")
            && provider_rpc.contains("async fn invoke"),
        "provider RPC must be async"
    );

    let state_manifest = read_repo_file("packages/d2b-state/Cargo.toml");
    let state_async = read_repo_file("packages/d2b-state/src/tokio_api.rs");
    assert!(
        state_manifest.contains("tokio = [\"host-fs\", \"dep:tokio\"]")
            && state_manifest
                .contains("tokio = { workspace = true, features = [\"rt\"], optional = true }"),
        "state Tokio adapters must remain explicit and optional"
    );
    assert!(
        state_async.contains("tokio::task::spawn_blocking")
            && !state_async.contains("std::fs::")
            && !state_async.contains("thread::sleep"),
        "sync kernel state APIs must be isolated behind spawn_blocking"
    );
    assert!(
        read_repo_file("packages/d2b-state/src/lib.rs").contains(
            "#[cfg(all(feature = \"host-fs\", not(target_os = \"linux\")))]\ncompile_error!(\"the host-fs feature requires Linux\");"
        ),
        "state host filesystem/OFD-lock support must fail explicitly off Linux"
    );
}
