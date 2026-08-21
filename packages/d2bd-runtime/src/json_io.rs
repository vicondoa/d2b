//! Provider-neutral bounded JSON artifact loading helpers.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::typed_error::TypedError;

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

pub fn load_json<T>(path: &Path) -> Result<T, TypedError>
where
    T: for<'de> Deserialize<'de>,
{
    let bytes = fs::read(path).map_err(|err| TypedError::InternalIo {
        context: format!("read {}", path.display()),
        detail: err.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|err| TypedError::InternalIo {
        context: format!("decode {}", path.display()),
        detail: err.to_string(),
    })
}

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
        })
}

pub fn read_trimmed_file(path: &Path, context: &str) -> Result<String, TypedError> {
    fs::read_to_string(path)
        .map(|content| content.trim().to_owned())
        .map_err(|err| TypedError::InternalIo {
            context: context.to_owned(),
            detail: err.to_string(),
        })
}
