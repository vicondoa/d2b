#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use serde::{Deserialize, Serialize};

const INVENTORY_SCHEMA_VERSION: u32 = 1;

/// The only host values that a build action may inherit.
///
/// Both values are supplied by the execution sandbox rather than by the
/// contributor's shell. In particular, HOME, PWD, and tool-selection
/// variables are deliberately absent: tools belong in the declared
/// toolchain/data annotations.
const ACTION_ENV_ALLOWLIST: &[&str] = &["PATH", "TMPDIR"];

pub(crate) const GENERATED_ARTIFACT_PATH: &str = "bazel/generated/action-network-policy.json";

pub(crate) fn pinned_action_env_allowlist() -> &'static [&'static str] {
    ACTION_ENV_ALLOWLIST
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Hub {
    Product,
    Walker,
}

impl Hub {
    pub(crate) const ALL: [Self; 2] = [Self::Product, Self::Walker];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Walker => "walker",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CrateKey {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: String,
}

impl CrateKey {
    pub(crate) fn new(name: &str, version: &str, source: &str) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
            source: source.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct RequiredAnnotations {
    pub(crate) build_script_env: BTreeMap<String, String>,
    pub(crate) build_script_data: BTreeSet<String>,
    pub(crate) build_script_tools: BTreeSet<String>,
    pub(crate) build_script_toolchains: BTreeSet<String>,
    pub(crate) build_script_use_cc_toolchain: bool,
    pub(crate) build_script_use_default_shell_env: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackageInput {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: Option<String>,
    pub(crate) build_script_target: bool,
    pub(crate) required_annotations: Option<RequiredAnnotations>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct HubLockAttrs {
    pub(crate) lockfile: String,
    pub(crate) cargo_lockfile: String,
    pub(crate) skip_cargo_lockfile_overwrite: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HubInput {
    pub(crate) hub: Hub,
    pub(crate) lock_attrs: HubLockAttrs,
    pub(crate) packages: Vec<PackageInput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InventoryInput {
    pub(crate) hubs: Vec<HubInput>,
    pub(crate) observed_action_environment: BTreeSet<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct BuildScriptCrate {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: Option<String>,
    pub(crate) required_annotations: RequiredAnnotations,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct HubInventory {
    pub(crate) hub: Hub,
    #[serde(rename = "hub_lock_attrs")]
    pub(crate) lock_attrs: HubLockAttrs,
    pub(crate) build_script_crates: Vec<BuildScriptCrate>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct HermeticityInventory {
    pub(crate) schema_version: u32,
    pub(crate) hubs: Vec<HubInventory>,
    pub(crate) action_env_allowlist: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedArtifact {
    pub(crate) relative_path: &'static str,
    pub(crate) contents: String,
    pub(crate) inventory: HermeticityInventory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InventoryError {
    DuplicateHub {
        hub: String,
    },
    MissingHub {
        hub: String,
    },
    InvalidLockAttribute {
        hub: String,
        attribute: &'static str,
    },
    DuplicateCrate {
        hub: String,
        crate_id: CrateKey,
    },
    UnenumeratedBuildScriptCrate {
        hub: String,
        crate_id: CrateKey,
    },
    AnnotationWithoutBuildScript {
        hub: String,
        crate_id: CrateKey,
    },
    UnknownAnnotationCrate {
        hub: String,
        crate_id: CrateKey,
    },
    InvalidAnnotation {
        hub: String,
        crate_id: CrateKey,
        field: &'static str,
    },
    UnlistedActionEnvironment {
        variable: String,
    },
    InvalidMetadata(String),
    GeneratedArtifactDrift,
    Serialization(String),
}

impl fmt::Display for InventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateHub { hub } => write!(formatter, "duplicate hermeticity hub {hub:?}"),
            Self::MissingHub { hub } => write!(formatter, "missing hermeticity hub {hub:?}"),
            Self::InvalidLockAttribute { hub, attribute } => {
                write!(
                    formatter,
                    "hub {hub:?} has invalid lock attribute {attribute}"
                )
            }
            Self::DuplicateCrate { hub, crate_id } => write!(
                formatter,
                "hub {hub:?} enumerates crate {} {} more than once",
                crate_id.name, crate_id.version
            ),
            Self::UnenumeratedBuildScriptCrate { hub, crate_id } => write!(
                formatter,
                "hub {hub:?} has an unenumerated third-party build-script crate {} {}",
                crate_id.name, crate_id.version
            ),
            Self::AnnotationWithoutBuildScript { hub, crate_id } => write!(
                formatter,
                "hub {hub:?} records annotations for {} {} without a build-script target",
                crate_id.name, crate_id.version
            ),
            Self::UnknownAnnotationCrate { hub, crate_id } => write!(
                formatter,
                "hub {hub:?} records annotations for unknown crate {} {}",
                crate_id.name, crate_id.version
            ),
            Self::InvalidAnnotation {
                hub,
                crate_id,
                field,
            } => write!(
                formatter,
                "hub {hub:?} has invalid {field} annotation for {} {}",
                crate_id.name, crate_id.version
            ),
            Self::UnlistedActionEnvironment { variable } => write!(
                formatter,
                "action environment variable {variable:?} is not in the pinned allowlist"
            ),
            Self::InvalidMetadata(message) => {
                write!(formatter, "invalid Cargo metadata: {message}")
            }
            Self::GeneratedArtifactDrift => {
                formatter.write_str("generated hermeticity inventory drift")
            }
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "cannot serialize hermeticity inventory: {message}"
                )
            }
        }
    }
}

impl Error for InventoryError {}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    source: Option<String>,
    targets: Vec<CargoTarget>,
}

#[derive(Debug, Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
}

pub(crate) fn hub_from_cargo_metadata(
    hub: Hub,
    lock_attrs: HubLockAttrs,
    metadata: &str,
    annotations: &BTreeMap<CrateKey, RequiredAnnotations>,
) -> Result<HubInput, InventoryError> {
    let metadata: CargoMetadata = serde_json::from_str(metadata)
        .map_err(|error| InventoryError::InvalidMetadata(error.to_string()))?;
    let mut packages = Vec::with_capacity(metadata.packages.len());
    let mut seen = BTreeSet::new();

    for package in metadata.packages {
        let build_script_target = package
            .targets
            .iter()
            .any(|target| target.kind.iter().any(|kind| kind == "custom-build"));
        let required_annotations = package.source.as_deref().and_then(|source| {
            annotations
                .get(&CrateKey::new(&package.name, &package.version, source))
                .cloned()
        });

        if let Some(source) = package.source.as_deref() {
            let crate_id = CrateKey::new(&package.name, &package.version, source);
            if !seen.insert(crate_id.clone()) {
                return Err(InventoryError::DuplicateCrate {
                    hub: hub.as_str().to_owned(),
                    crate_id,
                });
            }
        }

        packages.push(PackageInput {
            name: package.name,
            version: package.version,
            source: package.source,
            build_script_target,
            required_annotations,
        });
    }

    for crate_id in annotations.keys() {
        if !seen.contains(crate_id) {
            return Err(InventoryError::UnknownAnnotationCrate {
                hub: hub.as_str().to_owned(),
                crate_id: crate_id.clone(),
            });
        }
    }

    Ok(HubInput {
        hub,
        lock_attrs,
        packages,
    })
}

pub(crate) fn build_hub_inventory(input: &HubInput) -> Result<HubInventory, InventoryError> {
    validate_lock_attrs(input.hub, &input.lock_attrs)?;
    let mut seen = BTreeSet::new();
    let mut build_script_crates = Vec::new();

    for package in &input.packages {
        let Some(source) = package.source.as_deref() else {
            if package.required_annotations.is_some() {
                return Err(InventoryError::AnnotationWithoutBuildScript {
                    hub: input.hub.as_str().to_owned(),
                    crate_id: CrateKey::new(&package.name, &package.version, "workspace"),
                });
            }
            continue;
        };

        let crate_id = CrateKey::new(&package.name, &package.version, source);
        if !seen.insert(crate_id.clone()) {
            return Err(InventoryError::DuplicateCrate {
                hub: input.hub.as_str().to_owned(),
                crate_id,
            });
        }

        if package.build_script_target {
            let Some(required_annotations) = package.required_annotations.clone() else {
                return Err(InventoryError::UnenumeratedBuildScriptCrate {
                    hub: input.hub.as_str().to_owned(),
                    crate_id,
                });
            };
            validate_annotations(input.hub, &crate_id, &required_annotations)?;
            build_script_crates.push(BuildScriptCrate {
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone(),
                required_annotations,
            });
        } else if package.required_annotations.is_some() {
            return Err(InventoryError::AnnotationWithoutBuildScript {
                hub: input.hub.as_str().to_owned(),
                crate_id,
            });
        }
    }

    build_script_crates.sort_by(|left, right| {
        (&left.name, &left.version, &left.source).cmp(&(&right.name, &right.version, &right.source))
    });

    Ok(HubInventory {
        hub: input.hub,
        lock_attrs: input.lock_attrs.clone(),
        build_script_crates,
    })
}

pub(crate) fn build_inventory(
    input: &InventoryInput,
) -> Result<HermeticityInventory, InventoryError> {
    for variable in &input.observed_action_environment {
        if !ACTION_ENV_ALLOWLIST.contains(&variable.as_str()) {
            return Err(InventoryError::UnlistedActionEnvironment {
                variable: variable.clone(),
            });
        }
    }

    let mut hubs = Vec::with_capacity(input.hubs.len());
    let mut seen = BTreeSet::new();
    for hub in &input.hubs {
        if !seen.insert(hub.hub) {
            return Err(InventoryError::DuplicateHub {
                hub: hub.hub.as_str().to_owned(),
            });
        }
        hubs.push(build_hub_inventory(hub)?);
    }

    for expected in Hub::ALL {
        if !seen.contains(&expected) {
            return Err(InventoryError::MissingHub {
                hub: expected.as_str().to_owned(),
            });
        }
    }
    hubs.sort_by_key(|hub| hub.hub);

    Ok(HermeticityInventory {
        schema_version: INVENTORY_SCHEMA_VERSION,
        hubs,
        action_env_allowlist: ACTION_ENV_ALLOWLIST
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
    })
}

pub(crate) fn generated_artifact(
    input: &InventoryInput,
) -> Result<GeneratedArtifact, InventoryError> {
    let inventory = build_inventory(input)?;
    let mut contents = serde_json::to_string_pretty(&inventory)
        .map_err(|error| InventoryError::Serialization(error.to_string()))?;
    contents.push('\n');
    Ok(GeneratedArtifact {
        relative_path: GENERATED_ARTIFACT_PATH,
        contents,
        inventory,
    })
}

pub(crate) fn check_generated_artifact(
    expected: &GeneratedArtifact,
    actual_contents: &str,
) -> Result<(), InventoryError> {
    if actual_contents == expected.contents {
        Ok(())
    } else {
        Err(InventoryError::GeneratedArtifactDrift)
    }
}

fn validate_lock_attrs(hub: Hub, attrs: &HubLockAttrs) -> Result<(), InventoryError> {
    for (attribute, value) in [
        ("lockfile", attrs.lockfile.as_str()),
        ("cargo_lockfile", attrs.cargo_lockfile.as_str()),
    ] {
        if value.is_empty()
            || value.starts_with('/')
            || value.split('/').any(|component| component == "..")
        {
            return Err(InventoryError::InvalidLockAttribute {
                hub: hub.as_str().to_owned(),
                attribute,
            });
        }
    }
    if !attrs.skip_cargo_lockfile_overwrite {
        return Err(InventoryError::InvalidLockAttribute {
            hub: hub.as_str().to_owned(),
            attribute: "skip_cargo_lockfile_overwrite",
        });
    }
    Ok(())
}

fn validate_annotations(
    hub: Hub,
    crate_id: &CrateKey,
    annotations: &RequiredAnnotations,
) -> Result<(), InventoryError> {
    if annotations.build_script_use_default_shell_env {
        return Err(InventoryError::InvalidAnnotation {
            hub: hub.as_str().to_owned(),
            crate_id: crate_id.clone(),
            field: "build_script_use_default_shell_env",
        });
    }
    if annotations
        .build_script_env
        .keys()
        .any(|variable| variable.is_empty())
    {
        return Err(InventoryError::InvalidAnnotation {
            hub: hub.as_str().to_owned(),
            crate_id: crate_id.clone(),
            field: "build_script_env",
        });
    }
    if annotations
        .build_script_data
        .iter()
        .chain(annotations.build_script_tools.iter())
        .chain(annotations.build_script_toolchains.iter())
        .any(String::is_empty)
    {
        return Err(InventoryError::InvalidAnnotation {
            hub: hub.as_str().to_owned(),
            crate_id: crate_id.clone(),
            field: "build_script_data_or_toolchain",
        });
    }
    Ok(())
}

pub(crate) fn inventory_seam() {}

pub const PATCHED_BAZEL_VERSION: &str = "8.6.0";
pub const DERIVATION_SHA256_METHOD: &str = "raw-drv-file-sha256";
pub const PATCHED_BAZEL_OUTPUT: &str = "pkgs/bazel-8.6.0-seccomp";
pub const PATCHED_BAZEL_PATCH: &str = "pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch";
pub const FIXED_POLICY_PATH: &str = "pkgs/bazel-8.6.0-seccomp/seccomp-policy.json";
pub const CAPABILITY_ABI: &str = "d2b-bazel-seccomp-abi-v1";
pub const ACTION_NETWORK: &str = "none";
pub const SANDBOX_STRATEGY: &str = "sandboxed";
pub const RUSTSEC_DATABASE_INPUT: &str = "flake.nix:advisoryDbGit";
pub const BAZEL_SOURCE_SHA256: &str =
    "5b6d9e0742331cd65edf1685f0064e9144c30a2737357932a5da920d8eb54aed";
pub const BAZEL_ARCHIVE_SHA256: &str =
    "13a84586429b6084b13bd5040d78deda58d523012151e71e7d4be0c63dd831f9";
pub const BAZEL_PATCH_SHA256: &str =
    "3293478bf69c07dd16963f7ab7efeb6e4afcbb274c6461a8a8d94a4b211f2786";
pub const BAZEL_POLICY_SHA256: &str =
    "b1ce0e3607e6888af24475cac8ca4ab59555a42fed791dcc6420fbb4434831a9";
pub const BAZEL_X86_NAR_SHA256: &str =
    "b57d32790554461844f240fb376e406ac36cdfec7b211f3d5968cc50f41cefba";
pub const BAZEL_X86_EXECUTABLE_SHA256: &str =
    "743147d39b56b4a18b9f794995bd333cb57534fbb406bc07e882bf6913603e3a";
pub const BAZEL_ARM_NAR_SHA256: &str =
    "618ea346831a892c9617a124722634098aea215b56d183ae1c47c54a7a1a3a91";
pub const BAZEL_ARM_EXECUTABLE_SHA256: &str =
    "a9f37bf61a755bcd833e9a95dbd4b60978b03156be71add606fabb5c91df90fb";

pub const GOVERNED_ACTION_KINDS: &[&str] = &[
    "stable:Rustc",
    "stable:RustcMetadata",
    "stable:Clippy",
    "stable:rustdoc",
    "stable:rustdoc-test-compile",
    "stable:rustdoc-test-run",
    "stable:rustfmt",
    "stable:unpretty",
    "stable:CargoBuildScript",
    "stable:repository",
    "stable:setup",
    "stable:test",
    "nightly:Rustc",
    "nightly:RustcMetadata",
    "nightly:Clippy",
    "nightly:rustdoc",
    "nightly:rustdoc-test-compile",
    "nightly:rustdoc-test-run",
    "nightly:rustfmt",
    "nightly:unpretty",
    "nightly:CargoBuildScript",
    "nightly:repository",
    "nightly:setup",
    "nightly:test",
];

pub const SOCKET_PLANTS: &[&str] = &[
    "action-network-ipv4",
    "action-network-ipv6",
    "action-network-netlink",
    "action-network-packet",
    "action-network-unix-pathname",
    "action-network-unix-abstract",
    "action-network-socketpair",
    "action-network-io-uring",
];

pub const INHERITED_DESCRIPTOR_PLANTS: &[&str] = &[
    "inherited-socket",
    "inherited-io-uring",
    "inherited-io-uring-sqpoll",
    "inherited-io-uring-registered-fixed-socket",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NativeBazelOutput {
    pub nar_sha256: String,
    pub executable_sha256: String,
}

pub const DENIED_SYSCALLS: &[&str] = &[
    "socket",
    "socketpair",
    "connect",
    "bind",
    "listen",
    "accept",
    "accept4",
    "sendto",
    "sendmsg",
    "sendmmsg",
    "recvfrom",
    "recvmsg",
    "recvmmsg",
    "shutdown",
    "getsockname",
    "getpeername",
    "setsockopt",
    "getsockopt",
    "pidfd_getfd",
    "io_uring_setup",
    "io_uring_enter",
    "io_uring_register",
    "socketcall",
];

pub const PTRACE_REQUESTS: &[&str] = &[
    "PTRACE_TRACEME",
    "PTRACE_SETOPTIONS",
    "PTRACE_CONT",
    "PTRACE_DETACH",
];

pub const CONFIGURED_COVERAGE_TARGETS: &[&str] = &[
    "//ci/rust:api_census",
    "//ci/rust:fmt",
    "//ci/rust:clippy",
    "//ci/rust:main_tests",
    "//ci/rust:main_doctests",
    "//ci/rust:main_harness_free",
    "//ci/rust:broker_default",
    "//ci/rust:broker_layer1",
    "//ci/rust:broker_fakebackends",
    "//ci/rust:guest_shell_runner",
    "//ci/rust:no_bash_ast",
    "//ci/rust:schema_reproducibility",
    "//ci/rust:stub_no_socket",
    "//ci/rust:pinned_test_inventory",
    "//ci/rust:deny_main",
    "//ci/rust:deny_broker",
    "//ci/rust:deny_guest",
    "//ci/rust:audit_main",
    "//ci/rust:audit_broker",
    "//ci/rust:audit_guest",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActionNetworkInventory {
    pub action_network: String,
    pub sandbox_provider: String,
    pub derivation_sha256_method: String,
    pub capability_abi: String,
    pub fixed_policy: String,
    pub bazel_source_sha256: String,
    pub bazel_archive_sha256: String,
    pub patch_sha256: String,
    pub policy_sha256: String,
    pub output_nar_sha256: String,
    pub executable_sha256: String,
    pub native_outputs: BTreeMap<String, NativeBazelOutput>,
    pub load_point: String,
    pub configured_targets: Vec<String>,
    pub coverage_targets: Vec<String>,
    pub aquery_actions: Vec<String>,
    pub strategy_inventory: BTreeMap<String, String>,
    pub declared_inputs: BTreeSet<String>,
    pub denied_syscalls: Vec<String>,
    pub ptrace_requests: Vec<String>,
    pub socket_plants: Vec<String>,
    pub inherited_descriptor_plants: Vec<String>,
    pub repository_fetches_outside_actions: bool,
    pub fallback_strategies: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionNetworkError {
    MissingActionKind(String),
    MissingConfiguredTarget(String),
    MissingAqueryAction(String),
    WrongStrategy { action: String, strategy: String },
    ForbiddenStrategy(String),
    NetworkTag(String),
    MissingInput(String),
    MissingPlant(String),
    MissingInheritedPlant(String),
    RepositoryFetchInsideAction,
    FallbackPresent(String),
    WrongSandboxProvider,
    WrongCapabilityAbi,
    WrongLoadPoint,
    MalformedToolchainRecord,
}

impl fmt::Display for ActionNetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingActionKind(kind) => {
                write!(formatter, "action inventory is missing {kind}")
            }
            Self::MissingConfiguredTarget(kind) => {
                write!(formatter, "configured-target inventory is missing {kind}")
            }
            Self::MissingAqueryAction(kind) => {
                write!(formatter, "aquery inventory is missing {kind}")
            }
            Self::WrongStrategy { action, strategy } => {
                write!(
                    formatter,
                    "governed action {action} selected non-sandbox strategy {strategy}"
                )
            }
            Self::ForbiddenStrategy(strategy) => {
                write!(formatter, "forbidden Bazel strategy: {strategy}")
            }
            Self::NetworkTag(tag) => write!(formatter, "network-enabling action tag: {tag}"),
            Self::MissingInput(input) => {
                write!(formatter, "governed action input is not declared: {input}")
            }
            Self::MissingPlant(plant) => {
                write!(formatter, "action-network plant is missing: {plant}")
            }
            Self::MissingInheritedPlant(plant) => {
                write!(formatter, "inherited-descriptor plant is missing: {plant}")
            }
            Self::RepositoryFetchInsideAction => {
                formatter.write_str("repository fetch is inside a governed action")
            }
            Self::FallbackPresent(strategy) => {
                write!(formatter, "sandbox fallback is present: {strategy}")
            }
            Self::WrongSandboxProvider => {
                formatter.write_str("Bazel is not the pinned patched Nix output")
            }
            Self::WrongCapabilityAbi => formatter.write_str("Bazel capability ABI is not pinned"),
            Self::WrongLoadPoint => {
                formatter.write_str("sandbox filter load point is not before action exec")
            }
            Self::MalformedToolchainRecord => {
                formatter.write_str("pinned Bazel toolchain record is incomplete")
            }
        }
    }
}

impl Error for ActionNetworkError {}

pub fn complete_action_network_inventory() -> ActionNetworkInventory {
    let action_kinds = GOVERNED_ACTION_KINDS
        .iter()
        .map(|kind| (*kind).to_owned())
        .collect::<Vec<_>>();
    let strategy_inventory = action_kinds
        .iter()
        .map(|kind| (kind.clone(), SANDBOX_STRATEGY.to_owned()))
        .collect::<BTreeMap<_, _>>();
    ActionNetworkInventory {
        action_network: ACTION_NETWORK.to_owned(),
        sandbox_provider: PATCHED_BAZEL_OUTPUT.to_owned(),
        derivation_sha256_method: DERIVATION_SHA256_METHOD.to_owned(),
        capability_abi: CAPABILITY_ABI.to_owned(),
        fixed_policy: FIXED_POLICY_PATH.to_owned(),
        bazel_source_sha256: BAZEL_SOURCE_SHA256.to_owned(),
        bazel_archive_sha256: BAZEL_ARCHIVE_SHA256.to_owned(),
        patch_sha256: BAZEL_PATCH_SHA256.to_owned(),
        policy_sha256: BAZEL_POLICY_SHA256.to_owned(),
        output_nar_sha256: BAZEL_X86_NAR_SHA256.to_owned(),
        executable_sha256: BAZEL_X86_EXECUTABLE_SHA256.to_owned(),
        native_outputs: BTreeMap::from([
            (
                "x86_64-linux".to_owned(),
                NativeBazelOutput {
                    nar_sha256: BAZEL_X86_NAR_SHA256.to_owned(),
                    executable_sha256: BAZEL_X86_EXECUTABLE_SHA256.to_owned(),
                },
            ),
            (
                "aarch64-linux".to_owned(),
                NativeBazelOutput {
                    nar_sha256: BAZEL_ARM_NAR_SHA256.to_owned(),
                    executable_sha256: BAZEL_ARM_EXECUTABLE_SHA256.to_owned(),
                },
            ),
        ]),
        load_point: "after-sandbox-construction-before-action-command-exec".to_owned(),
        configured_targets: action_kinds.clone(),
        coverage_targets: CONFIGURED_COVERAGE_TARGETS
            .iter()
            .map(|target| (*target).to_owned())
            .collect(),
        aquery_actions: action_kinds,
        strategy_inventory,
        declared_inputs: BTreeSet::from([
            "packages/Cargo.lock".to_owned(),
            "bazel/supply_chain/yanked-snapshot.json".to_owned(),
            RUSTSEC_DATABASE_INPUT.to_owned(),
            "Cargo.lock:registry-checksum".to_owned(),
            "Cargo.lock:wl-proxy-rev+archive-sha256".to_owned(),
        ]),
        denied_syscalls: DENIED_SYSCALLS
            .iter()
            .map(|syscall| (*syscall).to_owned())
            .collect(),
        ptrace_requests: PTRACE_REQUESTS
            .iter()
            .map(|request| (*request).to_owned())
            .collect(),
        socket_plants: SOCKET_PLANTS
            .iter()
            .map(|plant| (*plant).to_owned())
            .collect(),
        inherited_descriptor_plants: INHERITED_DESCRIPTOR_PLANTS
            .iter()
            .map(|plant| (*plant).to_owned())
            .collect(),
        repository_fetches_outside_actions: true,
        fallback_strategies: Vec::new(),
    }
}

pub fn validate_action_network_inventory(
    inventory: &ActionNetworkInventory,
) -> Result<(), ActionNetworkError> {
    if inventory.action_network != ACTION_NETWORK {
        return Err(ActionNetworkError::NetworkTag(
            inventory.action_network.clone(),
        ));
    }
    if inventory.sandbox_provider != PATCHED_BAZEL_OUTPUT {
        return Err(ActionNetworkError::WrongSandboxProvider);
    }
    if inventory.derivation_sha256_method != DERIVATION_SHA256_METHOD {
        return Err(ActionNetworkError::WrongSandboxProvider);
    }
    if inventory.capability_abi != CAPABILITY_ABI {
        return Err(ActionNetworkError::WrongCapabilityAbi);
    }
    if inventory.bazel_source_sha256 != BAZEL_SOURCE_SHA256
        || inventory.bazel_archive_sha256 != BAZEL_ARCHIVE_SHA256
        || inventory.patch_sha256 != BAZEL_PATCH_SHA256
        || inventory.policy_sha256 != BAZEL_POLICY_SHA256
        || inventory.output_nar_sha256 != BAZEL_X86_NAR_SHA256
        || inventory.executable_sha256 != BAZEL_X86_EXECUTABLE_SHA256
    {
        return Err(ActionNetworkError::WrongSandboxProvider);
    }
    let native_x86 = inventory.native_outputs.get("x86_64-linux");
    let native_arm = inventory.native_outputs.get("aarch64-linux");
    if native_x86.is_none_or(|output| {
        output.nar_sha256 != BAZEL_X86_NAR_SHA256
            || output.executable_sha256 != BAZEL_X86_EXECUTABLE_SHA256
    }) || native_arm.is_none_or(|output| {
        output.nar_sha256 != BAZEL_ARM_NAR_SHA256
            || output.executable_sha256 != BAZEL_ARM_EXECUTABLE_SHA256
    }) {
        return Err(ActionNetworkError::WrongSandboxProvider);
    }
    if inventory.load_point != "after-sandbox-construction-before-action-command-exec" {
        return Err(ActionNetworkError::WrongLoadPoint);
    }
    for kind in GOVERNED_ACTION_KINDS {
        if !inventory.configured_targets.iter().any(|item| item == kind) {
            return Err(ActionNetworkError::MissingConfiguredTarget(
                (*kind).to_owned(),
            ));
        }
        for target in CONFIGURED_COVERAGE_TARGETS {
            if !inventory.coverage_targets.iter().any(|item| item == target) {
                return Err(ActionNetworkError::MissingConfiguredTarget(
                    (*target).to_owned(),
                ));
            }
        }
        if !inventory.aquery_actions.iter().any(|item| item == kind) {
            return Err(ActionNetworkError::MissingAqueryAction((*kind).to_owned()));
        }
        let strategy = inventory
            .strategy_inventory
            .get(*kind)
            .ok_or_else(|| ActionNetworkError::MissingActionKind((*kind).to_owned()))?;
        if strategy != SANDBOX_STRATEGY {
            return Err(ActionNetworkError::WrongStrategy {
                action: (*kind).to_owned(),
                strategy: strategy.clone(),
            });
        }
    }
    for strategy in inventory.strategy_inventory.values() {
        if matches!(
            strategy.as_str(),
            "process" | "local" | "standalone" | "worker" | "remote" | "no-sandbox"
        ) {
            return Err(ActionNetworkError::ForbiddenStrategy(strategy.clone()));
        }
    }
    if let Some(fallback) = inventory.fallback_strategies.first() {
        return Err(ActionNetworkError::FallbackPresent(fallback.clone()));
    }
    for tag in ["network", "requires-network", "net"] {
        if inventory.strategy_inventory.contains_key(tag) {
            return Err(ActionNetworkError::NetworkTag(tag.to_owned()));
        }
    }
    for input in [
        "packages/Cargo.lock",
        "bazel/supply_chain/yanked-snapshot.json",
        RUSTSEC_DATABASE_INPUT,
        "Cargo.lock:registry-checksum",
        "Cargo.lock:wl-proxy-rev+archive-sha256",
    ] {
        if !inventory.declared_inputs.contains(input) {
            return Err(ActionNetworkError::MissingInput(input.to_owned()));
        }
    }
    for syscall in DENIED_SYSCALLS {
        if !inventory.denied_syscalls.iter().any(|item| item == syscall) {
            return Err(ActionNetworkError::MissingInput((*syscall).to_owned()));
        }
    }
    for request in PTRACE_REQUESTS {
        if !inventory.ptrace_requests.iter().any(|item| item == request) {
            return Err(ActionNetworkError::MissingInput((*request).to_owned()));
        }
    }
    for plant in SOCKET_PLANTS {
        if !inventory.socket_plants.iter().any(|item| item == plant) {
            return Err(ActionNetworkError::MissingPlant((*plant).to_owned()));
        }
    }
    for plant in INHERITED_DESCRIPTOR_PLANTS {
        if !inventory
            .inherited_descriptor_plants
            .iter()
            .any(|item| item == plant)
        {
            return Err(ActionNetworkError::MissingInheritedPlant(
                (*plant).to_owned(),
            ));
        }
    }
    if !inventory.repository_fetches_outside_actions {
        return Err(ActionNetworkError::RepositoryFetchInsideAction);
    }
    Ok(())
}

pub fn action_network_json() -> Result<String, ActionNetworkError> {
    let inventory = complete_action_network_inventory();
    validate_action_network_inventory(&inventory)?;
    serde_json::to_string_pretty(&inventory)
        .map(|mut contents| {
            contents.push('\n');
            contents
        })
        .map_err(|_| ActionNetworkError::WrongLoadPoint)
}

pub fn validate_pinned_toolchain_record(record: &str) -> Result<(), ActionNetworkError> {
    let value: serde_json::Value =
        serde_json::from_str(record).map_err(|_| ActionNetworkError::MalformedToolchainRecord)?;
    let version = value.get("version").and_then(serde_json::Value::as_str);
    let systems = value
        .get("supportedSystems")
        .and_then(serde_json::Value::as_array)
        .map(|systems| {
            systems
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<BTreeSet<_>>()
        });
    if version != Some(PATCHED_BAZEL_VERSION)
        || value.get("artifact").and_then(serde_json::Value::as_str) != Some("bazel-8.6.0-seccomp")
        || value
            .get("derivationSha256Method")
            .and_then(serde_json::Value::as_str)
            != Some(DERIVATION_SHA256_METHOD)
        || systems != Some(BTreeSet::from(["x86_64-linux", "aarch64-linux"]))
        || value
            .get("patch")
            .and_then(|patch| patch.get("path"))
            .and_then(serde_json::Value::as_str)
            != Some(PATCHED_BAZEL_PATCH)
        || value
            .get("policy")
            .and_then(|policy| policy.get("path"))
            .and_then(serde_json::Value::as_str)
            != Some(FIXED_POLICY_PATH)
        || value
            .get("policy")
            .and_then(|policy| policy.get("capabilityAbi"))
            .and_then(serde_json::Value::as_str)
            != Some(CAPABILITY_ABI)
        || value
            .get("actionNetwork")
            .and_then(|action| action.get("strategy"))
            .and_then(serde_json::Value::as_str)
            != Some(SANDBOX_STRATEGY)
        || value
            .get("policy")
            .and_then(|policy| policy.get("noNetwork"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || value
            .get("policy")
            .and_then(|policy| policy.get("noFallback"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Err(ActionNetworkError::MalformedToolchainRecord);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn annotations() -> RequiredAnnotations {
        RequiredAnnotations {
            build_script_env: BTreeMap::new(),
            build_script_data: BTreeSet::new(),
            build_script_tools: BTreeSet::new(),
            build_script_toolchains: BTreeSet::new(),
            build_script_use_cc_toolchain: false,
            build_script_use_default_shell_env: false,
        }
    }

    fn package(
        name: &str,
        version: &str,
        source: Option<&str>,
        build_script_target: bool,
        required_annotations: Option<RequiredAnnotations>,
    ) -> PackageInput {
        PackageInput {
            name: name.to_owned(),
            version: version.to_owned(),
            source: source.map(str::to_owned),
            build_script_target,
            required_annotations,
        }
    }

    fn hub(hub: Hub, packages: Vec<PackageInput>) -> HubInput {
        HubInput {
            hub,
            lock_attrs: HubLockAttrs {
                lockfile: format!("bazel/cargo/{}.lock", hub.as_str()),
                cargo_lockfile: if hub == Hub::Product {
                    "packages/Cargo.lock".to_owned()
                } else {
                    "tests/tools/no-bash-ast-walker/Cargo.lock".to_owned()
                },
                skip_cargo_lockfile_overwrite: true,
            },
            packages,
        }
    }

    fn input(hubs: Vec<HubInput>) -> InventoryInput {
        InventoryInput {
            hubs,
            observed_action_environment: BTreeSet::from(["PATH".to_owned(), "TMPDIR".to_owned()]),
        }
    }

    fn complete_input() -> InventoryInput {
        let mut hubs = Vec::new();
        for hub_name in Hub::ALL.iter().rev() {
            hubs.push(hub(
                *hub_name,
                vec![
                    package(
                        "zeta",
                        "2.0.0",
                        Some("registry+https://example.invalid/index"),
                        true,
                        Some(annotations()),
                    ),
                    package(
                        "alpha",
                        "1.0.0",
                        Some("registry+https://example.invalid/index"),
                        true,
                        Some(annotations()),
                    ),
                    package("first-party", "1.0.0", None, true, None),
                    package(
                        "ordinary",
                        "1.0.0",
                        Some("registry+https://example.invalid/index"),
                        false,
                        None,
                    ),
                ],
            ));
        }
        input(hubs)
    }

    #[test]
    fn inventory_is_sorted_and_contains_only_third_party_build_scripts() {
        let inventory = build_inventory(&complete_input()).expect("complete inventory");

        let hub_names = inventory
            .hubs
            .iter()
            .map(|hub| hub.hub.as_str())
            .collect::<Vec<_>>();
        assert_eq!(hub_names, ["product", "walker"]);

        for hub in inventory.hubs {
            let crates = hub
                .build_script_crates
                .iter()
                .map(|crate_info| crate_info.name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(crates, ["alpha", "zeta"]);
            assert!(
                hub.build_script_crates
                    .iter()
                    .all(|crate_info| crate_info.source.is_some())
            );
        }
    }

    #[test]
    fn cargo_metadata_enumerates_custom_build_targets() {
        let metadata = r#"{
            "packages": [
                {
                    "name": "build-me",
                    "version": "1.0.0",
                    "source": "registry+https://example.invalid/index",
                    "targets": [{"name": "build-script-build-me", "kind": ["custom-build"]}]
                },
                {
                    "name": "ordinary",
                    "version": "1.0.0",
                    "source": "registry+https://example.invalid/index",
                    "targets": [{"name": "ordinary", "kind": ["lib"]}]
                },
                {
                    "name": "local-build",
                    "version": "1.0.0",
                    "source": null,
                    "targets": [{"name": "build-script-local-build", "kind": ["custom-build"]}]
                }
            ]
        }"#;
        let annotations = BTreeMap::from([(
            CrateKey::new(
                "build-me",
                "1.0.0",
                "registry+https://example.invalid/index",
            ),
            annotations(),
        )]);

        let hub = hub_from_cargo_metadata(
            Hub::Product,
            HubLockAttrs {
                lockfile: "bazel/cargo/product.lock".to_owned(),
                cargo_lockfile: "packages/Cargo.lock".to_owned(),
                skip_cargo_lockfile_overwrite: true,
            },
            metadata,
            &annotations,
        )
        .expect("metadata inventory");

        let inventory = build_hub_inventory(&hub).expect("hub inventory");
        assert_eq!(inventory.build_script_crates.len(), 1);
        assert_eq!(inventory.build_script_crates[0].name, "build-me");
    }

    #[test]
    fn an_unenumerated_build_script_crate_fails_closed() {
        let mut input = complete_input();
        input
            .hubs
            .iter_mut()
            .find(|hub| hub.hub == Hub::Product)
            .expect("product hub")
            .packages[0]
            .required_annotations = None;

        let error = build_inventory(&input).expect_err("missing annotation must refuse");
        assert!(matches!(
            error,
            InventoryError::UnenumeratedBuildScriptCrate { ref hub, ref crate_id }
                if hub == "product" && crate_id.name == "zeta"
        ));
    }

    #[test]
    fn an_annotation_for_a_non_build_script_crate_fails_closed() {
        let mut input = complete_input();
        input
            .hubs
            .iter_mut()
            .find(|hub| hub.hub == Hub::Product)
            .expect("product hub")
            .packages[3]
            .required_annotations = Some(annotations());

        let error = build_inventory(&input).expect_err("extra annotation must refuse");
        assert!(matches!(
            error,
            InventoryError::AnnotationWithoutBuildScript { ref hub, ref crate_id }
                if hub == "product" && crate_id.name == "ordinary"
        ));
    }

    #[test]
    fn an_unlisted_action_environment_value_fails_closed() {
        let mut input = complete_input();
        input
            .observed_action_environment
            .insert("D2B_PLANTED_HOST_VALUE".to_owned());

        let error = build_inventory(&input).expect_err("unlisted environment must refuse");
        assert!(matches!(
            error,
            InventoryError::UnlistedActionEnvironment { ref variable }
                if variable == "D2B_PLANTED_HOST_VALUE"
        ));
    }

    #[test]
    fn generated_artifact_is_deterministic_and_has_the_generated_path() {
        let first = generated_artifact(&complete_input()).expect("first artifact");
        let second = generated_artifact(&complete_input()).expect("second artifact");

        assert_eq!(
            first.relative_path,
            "bazel/generated/action-network-policy.json"
        );
        assert_eq!(first.contents, second.contents);
        assert!(first.contents.ends_with('\n'));
        let json: serde_json::Value =
            serde_json::from_str(&first.contents).expect("valid inventory json");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(
            json["action_env_allowlist"],
            serde_json::json!(["PATH", "TMPDIR"])
        );
    }

    #[test]
    fn generated_drift_is_a_typed_error_for_the_generator_to_render() {
        let artifact = generated_artifact(&complete_input()).expect("artifact");
        let error = check_generated_artifact(&artifact, "{}\n")
            .expect_err("different generated bytes must refuse");

        assert!(matches!(error, InventoryError::GeneratedArtifactDrift));
        assert_eq!(error.to_string(), "generated hermeticity inventory drift");
    }
}
