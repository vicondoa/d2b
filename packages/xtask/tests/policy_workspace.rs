#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;

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
const PROVIDER_INTEGRATION_FILES: &[&str] = &[
    "docs/reference/daemon-api.md",
    "docs/reference/manifest-bundle.md",
    "docs/reference/schemas/v2/bundle.json",
    "docs/reference/schemas/v2/bundle.md",
    "docs/reference/schemas/v2/provider-registry-v2.json",
    "docs/reference/schemas/v2/provider-registry-v2.md",
    "flake.nix",
    "nixos-modules/bundle-artifacts.nix",
    "nixos-modules/bundle.nix",
    "nixos-modules/default.nix",
    "nixos-modules/provider-registry-v2-json.nix",
    "packages/Cargo.lock",
    "packages/Cargo.toml",
    "packages/d2b-contract-tests/tests/realm_workload_schema_contract.rs",
    "packages/d2b-contracts/src/lib.rs",
    "packages/d2b-contracts/src/provider_registry_v2.rs",
    "packages/d2b-core/fuzz/src/bin/core.rs",
    "packages/d2b-core/src/bundle.rs",
    "packages/d2b-core/src/bundle_resolver.rs",
    "packages/d2b-core/tests/bundle_resolver_runner_intent_parity.rs",
    "packages/d2b-priv-broker/src/ops/storage_contract.rs",
    "packages/d2b-priv-broker/src/ops/tap.rs",
    "packages/d2b-priv-broker/src/runtime.rs",
    "packages/d2b-priv-broker/tests/w15_install_migrate.rs",
    "packages/d2bd/Cargo.toml",
    "packages/d2bd/src/kernel_module_check.rs",
    "packages/d2bd/src/lib.rs",
    "packages/d2bd/src/net_vm_bundle_gate.rs",
    "packages/d2bd/src/provider_effects.rs",
    "packages/d2bd/src/provider_registry.rs",
    "packages/d2bd/src/storage_lifecycle.rs",
    "packages/d2bd/src/supervisor/stop_dag.rs",
    "packages/d2bd/tests/bundle_tampered_envelope.rs",
    "packages/d2bd/tests/common/mod.rs",
    "packages/d2bd/tests/public_status_socket.rs",
    "packages/xtask/src/main.rs",
    "packages/xtask/tests/policy_workspace.rs",
    "tests/unit/nix/cases/realm-workloads.nix",
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

    for package in W4_PROVIDER_CRATES {
        let manifest = read_repo_file(&format!("packages/{package}/Cargo.toml"));
        for required in [
            "d2b-contracts = { workspace = true, default-features = false, features = [\"v2-provider\"] }",
            "d2b-provider = { workspace = true, default-features = false }",
        ] {
            assert!(
                manifest.contains(required),
                "{package} must depend on the canonical provider boundary: {required}"
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

    for package in [
        W4_SUPPORT_CRATE,
        "d2b-provider-infrastructure-azure-vm",
        "d2b-provider-runtime-azure-vm",
    ] {
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
                "async-trait",
                "d2b-azure-vm-fake-sdk",
                "d2b-contracts",
                "d2b-provider",
                "d2b-provider-toolkit",
                "serde",
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

    for production_package in [
        "d2b-gateway",
        "d2b-gateway-runtime",
        "d2b-guestd",
        "d2b-userd",
        "d2bd",
    ] {
        let dependencies = transitive_package_names(&metadata, production_package);
        for forbidden in [
            "d2b-provider-infrastructure-azure-vm",
            "d2b-provider-runtime-azure-vm",
        ] {
            assert!(
                !dependencies.contains(forbidden),
                "{production_package} must not include unavailable Azure VM provider {forbidden}"
            );
        }
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
            && daemon.contains("provider_registry::compose_startup_registry(state, &artifact)")
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

    let effects = read_repo_file("packages/d2bd/src/provider_effects.rs");
    for forbidden in ["d2b_priv_broker", "std::process::Command", "Command::new"] {
        assert!(
            !effects.contains(forbidden),
            "daemon semantic effects must not bypass typed provider ports via {forbidden}"
        );
    }
    for required in [
        "dispatch_broker_vm_start",
        "dispatch_broker_vm_stop_as",
        "find_vm_start_intent",
        "find_runner_intent",
        "still_alive_same_start_time",
    ] {
        assert!(
            effects.contains(required),
            "live runtime adapter must invoke the existing daemon authority seam {required}"
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
