//! Fixed Provider/system-core manifest projection.

use serde::Serialize;

/// Empty Provider config and fixed component/resource declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemCoreManifest {
    /// Canonical artifact identity.
    pub artifact_id: &'static str,
    /// Empty closed config schema.
    pub config: serde_json::Value,
    /// Fixed in-process handlers.
    pub components: [&'static str; 2],
    /// ResourceTypes owned by this Provider.
    pub resource_types: [&'static str; 2],
    /// system-core owns no Provider state Volume.
    pub declares_state_volume: bool,
}

impl SystemCoreManifest {
    /// Return the canonical manifest.
    pub fn canonical() -> Self {
        Self {
            artifact_id: "system-core",
            config: serde_json::json!({}),
            components: ["host-controller", "user-controller"],
            resource_types: ["Host", "User"],
            declares_state_volume: false,
        }
    }

    /// Validate the fixed empty Provider config.
    pub fn validate_config(config: &serde_json::Value) -> Result<(), ManifestError> {
        if config.as_object().is_some_and(serde_json::Map::is_empty) {
            Ok(())
        } else {
            Err(ManifestError::ConfigNotEmpty)
        }
    }
}

/// Manifest admission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    /// Provider/system-core config must be an empty object.
    ConfigNotEmpty,
}

impl core::fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("system-core-config-not-empty")
    }
}

impl std::error::Error for ManifestError {}
