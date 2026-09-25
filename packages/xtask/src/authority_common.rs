//! Shared authority scaffolding: the recursive `.rs` collector and the
//! committed-artifact drift check the provider-crate-layout authorities all
//! use. One home instead of three byte-identical private copies.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Recursively collect every `.rs` file under one source tree.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn collect_rs_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Err(format!(
            "missing-src: the declaring crate has no src tree at {}",
            dir.display()
        ));
    }
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a source entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_rs_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    Ok(out)
}

/// Fail when the committed copy of one generated artifact differs from the
/// declarations' render. `authority` names the owning authority and is the
/// only per-caller difference between the former private copies.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn verify_committed(
    repo_root: &Path,
    relative: &str,
    rendered: &str,
    authority: &str,
) -> Result<(), String> {
    let artifact_path = repo_root.join(relative);
    let on_disk = fs::read_to_string(&artifact_path).map_err(|_| {
        format!(
            "{authority} artifact is missing at {}; run `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        )
    })?;
    if on_disk != rendered {
        return Err(format!(
            "{authority} drift: the committed generated artifact {} differs from the declarations' output; a hand edit or a stale generation must be repaired by `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        ));
    }
    Ok(())
}