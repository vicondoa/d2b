//! Provider-neutral bounded JSON artifact loading helpers.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::typed_error::{TypedError, error_source};

/// Resolve a bundle-relative artifact path within `base_dir`.
///
/// An absolute path is honored verbatim when it already exists; an
/// absolute path that points nowhere falls back to `base_dir` + its file
/// name, so a bundle self-reference keeps working after unpacking; a
/// relative path joins `base_dir` unchanged.
pub fn resolve_bundle_artifact_path(base_dir: &Path, raw_path: &str) -> PathBuf {
    let raw = Path::new(raw_path);
    if raw.is_absolute() && raw.exists() {
        raw.to_path_buf()
    } else if raw.is_absolute() {
        raw.file_name()
            .map(|name| base_dir.join(name))
            .unwrap_or_else(|| raw.to_path_buf())
    } else {
        base_dir.join(raw)
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "synchronous path"
)]
pub fn load_json<T>(path: &Path) -> Result<T, TypedError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: format!("read {}", path.display()),
        detail: err.to_string(),
        source: error_source(err),
    })?;
    serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalIo {
        context: format!("decode {}", path.display()),
        detail: err.to_string(),
        source: error_source(err),
    })
}

/// Load the bundle manifest as a JSON object, owning its top-level map.
///
/// # Errors
///
/// Returns `InternalIo` when the file cannot be read or decoded, or when
/// the root value is not an object (the manifest schema requires one).
pub fn load_manifest(
    path: &Path,
) -> Result<serde_json::Map<String, serde_json::Value>, TypedError> {
    let value: serde_json::Value = load_json(path)?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| TypedError::InternalIo {
            context: format!("decode manifest {}", path.display()),
            detail: "manifest must be a JSON object".to_owned(),
            source: None,
        })
}

#[allow(
    clippy::disallowed_methods,
    reason = "synchronous path"
)]
pub fn read_trimmed_file(path: &Path, context: &str) -> Result<String, TypedError> {
    fs::read_to_string(path)
        .map(|content| content.trim().to_owned())
        .map_err(|err| TypedError::InternalIo {
            context: context.to_owned(),
            detail: err.to_string(),
            source: error_source(err),
        })
}
