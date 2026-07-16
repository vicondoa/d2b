use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Top-level private bundle index installed beside the public vms.json manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bundle {
    /// Version of the bundle format; bundleVersion 12 adds the private
    /// provider-registry-v2 composition artifact while artifact schemaVersion
    /// stays v2.
    pub bundle_version: u32,
    /// Schema version directory used to validate all artifacts in this bundle.
    pub schema_version: String,
    /// Public vms.json manifest path retained for v0.4.0 compatibility.
    pub public_manifest_path: String,
    /// Private host.json artifact path.
    pub host_path: String,
    /// Private processes.json artifact path.
    pub processes_path: String,
    /// Private privileges.json artifact path.
    pub privileges_path: String,
    /// Private storage lifecycle artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_path: Option<String>,
    /// Private synchronization/lock contract artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_path: Option<String>,
    /// Private local-root allocator metadata artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocator_path: Option<String>,
    /// Private realm-controller metadata artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_controllers_path: Option<String>,
    /// Private realm identity metadata artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_identity_path: Option<String>,
    /// Argv-free provider-neutral launcher metadata served through the public API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_workloads_launcher_v2_path: Option<String>,
    /// Private configured unsafe-local workload metadata artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsafe_local_workloads_path: Option<String>,
    /// Private canonical provider descriptors and opaque daemon intent mappings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_registry_v2_path: Option<String>,
    /// Per-VM closure artifact paths keyed by VM name.
    pub closures: Vec<BundleClosureRef>,
    /// Minijail profile metadata paths shipped with the bundle.
    pub minijail_profiles: Vec<BundleProfileRef>,
    /// Managed SSH key metadata used by the CLI/daemon key inventory surface.
    #[serde(default)]
    pub managed_keys: BundleManagedKeys,
    /// Generation metadata for auditing drift between Nix output and runtime state.
    pub generation: BundleGeneration,
    /// SHA-256 self-hash of the bundle JSON with `bundleHash` absent and
    /// `artifactHashes` nullified.
    ///
    /// Emitted by `nixos-modules/bundle.nix` as `"sha256:<hex64>"`.
    /// The Rust loader verifies it by stripping `bundleHash`, setting
    /// `artifactHashes` to null, re-serialising with serde_json (sorted
    /// keys, no spaces), and comparing.  On `schemaVersion "v2"` bundles
    /// a missing field is a hard failure; on v1 it logs a warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_hash: Option<String>,

    /// Per-artifact SHA-256 hashes for tamper detection of every private
    /// bundle artifact loaded by the resolver.
    ///
    /// Keys match the path strings stored in the bundle path fields: absolute
    /// paths for `host_path`, `processes_path`, `privileges_path`,
    /// `storage_path`, `sync_path`, `allocator_path`,
    /// `realm_controllers_path`, `realm_identity_path`,
    /// `realm_workloads_launcher_v2_path`, `unsafe_local_workloads_path`, and
    /// `provider_registry_v2_path`;
    /// bundle-relative paths for `closures[*].path` and
    /// `minijail_profiles[*].path`. Values are
    /// `"sha256:<hex64>"` strings.
    ///
    /// When `None`, per-artifact hash verification is skipped (backwards
    /// compatibility with bundles that pre-date this field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hashes: Option<BTreeMap<String, String>>,
}

/// Bundle-scoped managed SSH key metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct BundleManagedKeys {
    #[serde(default = "default_keys_dir")]
    pub keys_dir: String,
    #[serde(default = "default_known_hosts_path")]
    pub known_hosts_path: String,
    #[serde(default)]
    pub overrides: Vec<BundleManagedKeyOverride>,
}

impl Default for BundleManagedKeys {
    fn default() -> Self {
        Self {
            keys_dir: default_keys_dir(),
            known_hosts_path: default_known_hosts_path(),
            overrides: Vec::new(),
        }
    }
}

impl BundleManagedKeys {
    pub fn effective_key_path(&self, vm: &str) -> PathBuf {
        self.overrides
            .iter()
            .find(|entry| entry.vm == vm)
            .map(|entry| PathBuf::from(&entry.key_path))
            .unwrap_or_else(|| PathBuf::from(&self.keys_dir).join(format!("{vm}_ed25519")))
    }

    pub fn public_key_path(&self, vm: &str) -> PathBuf {
        let mut path = self.effective_key_path(vm).into_os_string();
        path.push(".pub");
        PathBuf::from(path)
    }

    pub fn known_hosts_path_buf(&self) -> PathBuf {
        PathBuf::from(&self.known_hosts_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleManagedKeyOverride {
    pub vm: String,
    pub key_path: String,
}

fn default_keys_dir() -> String {
    "/var/lib/d2b/keys".to_owned()
}

fn default_known_hosts_path() -> String {
    "/var/lib/d2b/known_hosts.d2b".to_owned()
}

/// Reference to a per-VM closure artifact under closures/<vm>.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleClosureRef {
    /// VM name whose closure metadata is described.
    pub vm: String,
    /// Bundle-relative path to the VM closure JSON.
    pub path: String,
}

/// Reference to typed minijail profile metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleProfileRef {
    /// Stable profile identifier used by processes.json roles.
    pub profile_id: String,
    /// Bundle-relative path to the profile JSON.
    pub path: String,
}

/// Generator identity and timestamps used by bundle drift gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleGeneration {
    /// Tool or module that emitted the bundle.
    pub generator: String,
    /// Optional source revision or derivation identity.
    pub source_revision: Option<String>,
    /// Reproducible timestamp string supplied by the Nix emitter.
    pub generated_at: Option<String>,
}
