//! ResourceType schema and Nix-option generator.
//!
//! Rust DTOs are the source of truth.  The generated JSON files are the
//! committed public schema contract; the generated Nix modules are a
//! deliberately thin option boundary that preserves the same field names and
//! delegates cross-resource references to the Zone validation module.

use std::{
    fs,
    path::{Path, PathBuf},
};

use d2b_contracts::v3::{
    EndpointSpec, EphemeralProcessSpec, GuestSpec, HostSpec, UserSpec, process::ProcessSpec,
};
use schemars::schema_for;
use serde_json::Value;

/// Generate primitive ResourceType schemas and Nix option modules.
pub fn generate(repo_root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let schema_dir = repo_root.join("docs/reference/schemas/v3");
    let nix_dir = repo_root.join("nixos-modules/resource-schemas");
    fs::create_dir_all(&schema_dir)?;
    fs::create_dir_all(&nix_dir)?;
    let types = [
        "Host",
        "Guest",
        "Process",
        "EphemeralProcess",
        "User",
        "Endpoint",
    ];
    let mut written = Vec::with_capacity(types.len() * 2);
    for name in types {
        let schema_path = schema_dir.join(format!("{name}.schema.json"));
        // The existing Zone-schema generator owns the committed JSON files.
        // Reuse those authoritative artifacts when present; the Rust DTO
        // fallback keeps this command useful for a clean bootstrap tree
        // without creating a second conflicting generator.
        if !schema_path.exists() {
            let schema: Value = match name {
                "Host" => serde_json::to_value(schema_for!(HostSpec))?,
                "Guest" => serde_json::to_value(schema_for!(GuestSpec))?,
                "Process" => serde_json::to_value(schema_for!(ProcessSpec))?,
                "EphemeralProcess" => serde_json::to_value(schema_for!(EphemeralProcessSpec))?,
                "User" => serde_json::to_value(schema_for!(UserSpec))?,
                "Endpoint" => serde_json::to_value(schema_for!(EndpointSpec))?,
                _ => unreachable!("closed schema list"),
            };
            let bytes = serde_json::to_vec_pretty(&schema)?;
            fs::write(&schema_path, [bytes.as_slice(), b"\n"].concat())?;
        }
        written.push(schema_path);

        let nix_path = nix_dir.join(format!("{name}.nix"));
        let source = match name {
            "EphemeralProcess" => "ephemeral_process",
            "Host" => "host",
            "Guest" => "guest",
            "Process" => "process",
            "User" => "user",
            "Endpoint" => "endpoint",
            _ => unreachable!("closed schema list"),
        };
        let module = format!(
            "# Generated from packages/d2b-contracts/src/v3/{lower}.rs.\n\
             # Do not hand-edit; run xtask gen-resource-schemas.\n\
             {{ lib }}:\n\
             {{\n\
               type = \"{name}\";\n\
               schema = builtins.fromJSON (builtins.readFile ../docs/reference/schemas/v3/{name}.schema.json);\n\
             }}\n",
            lower = source
        );
        fs::write(&nix_path, module)?;
        written.push(nix_path);
    }
    Ok(written)
}
