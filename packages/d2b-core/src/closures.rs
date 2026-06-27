use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-VM closure metadata consumed by the future daemon parity oracle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClosureMetadata {
    /// Schema version used by this artifact.
    pub schema_version: String,
    /// VM name this closure describes.
    pub vm: String,
    /// Declared top-level NixOS system path.
    pub toplevel: String,
    /// Complete closure paths required by the VM.
    pub closure_paths: Vec<String>,
    /// Nix DB registration dump for the closure.
    pub db_dump_path: String,
    /// Declared runner path from the public manifest/emitter.
    pub declared_runner: String,
    /// Runner path observed or snapshotted for parity checks.
    pub runner_parity_path: String,
    /// Whether declared runner and parity path agree.
    pub runner_parity_ok: bool,
    /// Generation and source metadata.
    pub generation: ClosureGeneration,
}

/// Generation metadata for a VM closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClosureGeneration {
    /// Host-side generation number when known.
    pub host_generation: Option<u64>,
    /// VM generation label or number when known.
    pub vm_generation: Option<String>,
    /// Source revision or derivation identity.
    pub source_revision: Option<String>,
    /// Reproducible timestamp string supplied by the Nix emitter.
    pub generated_at: Option<String>,
}
