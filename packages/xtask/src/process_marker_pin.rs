use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};

const PIN_REL: &str = "tests/golden/pinned/process-marker-legacy-paths.json";
const SCHEMA_VERSION: u32 = 1;
const FROZEN_UNIVERSE_SHA256: &str =
    "0f6899e939fd8e0b49f41b56a0221f33552d79348adc2853927d338e610f8f34";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessMarkerPin {
    schema_version: u32,
    active_paths: Vec<String>,
    retired_paths: Vec<String>,
}

pub fn check(repo_root: &Path) -> Result<(), String> {
    let bytes = fs::read(repo_root.join(PIN_REL))
        .map_err(|error| format!("cannot read {PIN_REL}: {error}"))?;
    let pin: ProcessMarkerPin = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot parse {PIN_REL}: {error}"))?;
    validate(&pin)
}

fn validate(pin: &ProcessMarkerPin) -> Result<(), String> {
    if pin.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "{PIN_REL}: unsupported schemaVersion {}; expected {SCHEMA_VERSION}",
            pin.schema_version
        ));
    }
    if pin.active_paths.is_empty() {
        return Err(format!("{PIN_REL}: activePaths must not be empty"));
    }
    ensure_sorted_unique("activePaths", &pin.active_paths)?;
    ensure_sorted_unique("retiredPaths", &pin.retired_paths)?;

    let mut universe = BTreeSet::new();
    for path in pin.active_paths.iter().chain(&pin.retired_paths) {
        ensure_safe_relative_path(path)?;
        if !universe.insert(path.as_str()) {
            return Err(format!(
                "{PIN_REL}: path appears in both activePaths and retiredPaths"
            ));
        }
    }

    let mut canonical = String::new();
    for path in universe {
        canonical.push_str(path);
        canonical.push('\n');
    }
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    if digest != FROZEN_UNIVERSE_SHA256 {
        return Err(format!(
            "{PIN_REL}: path universe changed; legacy exemptions may only move from activePaths \
             to retiredPaths"
        ));
    }
    Ok(())
}

fn ensure_sorted_unique(label: &str, paths: &[String]) -> Result<(), String> {
    for adjacent in paths.windows(2) {
        if adjacent[0] >= adjacent[1] {
            return Err(format!(
                "{PIN_REL}: {label} must be strictly sorted with no duplicates"
            ));
        }
    }
    Ok(())
}

fn ensure_safe_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{PIN_REL}: every entry must be a non-empty normalized relative path"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_pin() -> ProcessMarkerPin {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask has a repository parent");
        let bytes = fs::read(repo_root.join(PIN_REL)).expect("read committed pin");
        serde_json::from_slice(&bytes).expect("parse committed pin")
    }

    #[test]
    fn committed_pin_is_valid() {
        validate(&committed_pin()).expect("committed process-marker pin is valid");
    }

    #[test]
    fn adding_an_active_path_cannot_expand_the_frozen_universe() {
        let mut pin = committed_pin();
        pin.active_paths
            .push("packages/new-legacy-marker.rs".to_string());
        pin.active_paths.sort();
        let error = validate(&pin).expect_err("an added exemption must fail");
        assert!(error.contains("path universe changed"), "{error}");
    }

    #[test]
    fn retiring_an_active_path_preserves_the_frozen_universe() {
        let mut pin = committed_pin();
        let retired = pin.active_paths.remove(0);
        pin.retired_paths.push(retired);
        pin.retired_paths.sort();
        validate(&pin).expect("moving an exemption to retiredPaths is shrink-only");
    }
}
