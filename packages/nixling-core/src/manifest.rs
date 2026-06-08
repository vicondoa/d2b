//! Bundle / manifest API for the broker's ValidateBundle dispatch.
//!
//! W4-fu clean-break: the broker reads the bundle manifest from
//! its server-configured `bundle_path` and runs a strict
//! [`ManifestV04`] parse against it. The bootstrap path keeps a
//! looser "file exists" check in `crate::bootstrap::manifest`
//! because the W2 probe-* test harnesses pre-date the v0.4
//! manifest schema.
//!
//! Future W4-fu-fu work: extend this surface with a
//! `BundleResolver` that maps `BundleOpId` opaque IDs to concrete
//! `NftIntent` / `RouteIntent` / `SysctlIntent` rows, so the
//! broker's live_handlers can be invoked with resolved plans
//! rather than just typed-Unimplemented envelopes.

use crate::manifest_v04::ManifestV04;
use std::path::Path;

/// Strict bundle manifest validator used by the broker's
/// `ValidateBundle` dispatch arm. Reads `path`, parses it as a
/// [`ManifestV04`], and returns a stable error string on failure.
///
/// The broker dispatches with `&config.bundle_path` (default
/// `/var/lib/nixling/current-bundle/manifest.json`); the daemon
/// never names a bundle path on the wire.
pub fn validate_bundle(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("bundle path does not exist: {}", path.display()));
    }
    ManifestV04::from_path(path).map_err(|err| format!("manifest parse failed: {err}"))?;
    Ok(())
}
