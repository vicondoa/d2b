use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Top-level private bundle index installed beside the public vms.json manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bundle {
    /// Version of the bundle format.
    pub bundle_version: u32,
    /// Schema version directory used to validate all artifacts in this bundle.
    pub schema_version: String,
    /// Private privileges.json artifact path.
    pub privileges_path: String,
    /// Private storage lifecycle artifact path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_path: Option<String>,
    /// Argv-free provider-neutral launcher metadata served through the public API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_workloads_launcher_v2_path: Option<String>,
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
    /// paths for `privileges_path`, `storage_path`, and
    /// `realm_workloads_launcher_v2_path`. Values are
    /// `"sha256:<hex64>"` strings.
    ///
    /// When `None`, per-artifact hash verification is skipped (backwards
    /// compatibility with bundles that pre-date this field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hashes: Option<BTreeMap<String, String>>,
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
