//! The per-crate resource-type declarations and the generated type authority.
//!
//! Every `packages/d2b-provider-*/resource-types.json` declares the resource
//! types its owning crate registers with the plane. This module:
//!
//! - aggregates those declarations into the `V3_CONVERTED_RESOURCE_TYPES`
//!   authority const the v3 resource plane consumes (`include!`d from
//!   `packages/d2b-contracts/src/identity.rs`), keeping the committed entry
//!   order the plane's closed-form fence has consumed since that const
//!   landed; a declared type absent from the committed order is appended
//!   after it in sorted order, so a new type needs no edit here;
//! - runs the declaration-to-descriptor parity gate: every crate's
//!   declaration and its registered descriptor (`resource_type:
//!   WellKnownType::...` or `descriptor(WellKnownType::...` in the crate's
//!   sources) must agree on the type name, a declaration that omits a type
//!   its descriptor registers fails naming both, and a type declared by two
//!   crates fails naming both crates;
//! - owns the drift gate over the generated artifact: a hand edit fails
//!   and regeneration is idempotent;
//! - derives the committed Nix type registry (`resource-types.nix`) and the
//!   per-type inventory (`resource-inventories.nix`) from the same
//!   declarations: the standard registry's set and the
//!   `coreSchemaPointers` keys come from the declared standard
//!   (unqualified) types, and the remaining inventory tables remain
//!   committed facts the renderer holds, since the declarations do not carry
//!   them; both files stay byte-identical to the committed ones and share
//!   the authority's drift and idempotence gates. The historical committed
//!   registry order is preserved by an order table, so a new declared type
//!   still needs no shared-crate edit (U14's zero-outside-edit fixture).
//!
//! The generator is wired into the existing policy check
//! (`cargo xtask check-provider-crate-layout`): the check runs the parity
//! and drift gates after the crate-layout check, and `--fix` regenerates the
//! artifacts. This keeps every gate in the suite the layout check already
//! runs, without editing `provider_crate_policy.rs`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// The directory-glob root the per-crate declarations live under.
const PACKAGES_DIR: &str = "packages";
const PROVIDER_PREFIX: &str = "d2b-provider-";
const DECLARATION_FILE: &str = "resource-types.json";

/// The repository-relative generated artifact path (relative to the source
/// file that `include!`s it,so `include!("generated/...")` resolves it).
pub(crate) const GENERATED_ARTIFACT: &str =
    "packages/d2b-contracts/src/generated/v3_converted_resource_types.rs";

/// The repository-relative generated Nix standard type registry.
pub(crate) const NIX_RESOURCE_TYPES_OUT: &str = "nixos-modules/generated/resource-types.nix";
/// The repository-relative generated Nix resource inventory.
pub(crate) const NIX_RESOURCE_INVENTORIES_OUT: &str =
    "nixos-modules/generated/resource-inventories.nix";

/// The committed order of the standard ResourceType registry the Nix
/// authoring surface consumes (`resource-types.nix`).
///
/// The order is the historical hand-written registry order, not derivable
/// from the per-crate declarations; this table preserves it, mirroring
/// [`COMMITTED_V3_ORDER`]. A declared standard type absent from this table is
/// appended after the committed block in sorted order, so adding a type
/// needs no shared-crate edit.
const COMMITTED_NIX_STANDARD_ORDER: &[&str] = &[
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "EmergencyPolicy",
    "Host",
    "Guest",
    "Process",
    "EphemeralProcess",
    "Volume",
    "VolumeBinding",
    "Network",
    "Device",
    "User",
    "Credential",
    "Endpoint",
    "ResourceExport",
    "ResourceImport",
    "Command",
    "Operation",
    "SeccompProfile",
];

/// The committed order of the type authority const.
///
/// The v3 plane's closed-form fence has consumed the const since it landed
/// and its fence test pins the member set, so the emitted order must stay
/// byte-identical to the committed one. The order is not derivable from
/// the per-crate declarations (it is the historical hand-written const
/// order); this table preserves it. A declared type absent from this table
/// is appended after the committed block in sorted order, so adding a
/// type needs no shared-crate edit (U14's zero-outside-edit fixture).
const COMMITTED_V3_ORDER: &[&str] = &[
    "Process",
    "EphemeralProcess",
    "Guest",
    "Volume",
    "VolumeBinding",
    "Endpoint",
    "Host",
    "User",
    "activation-nixos.d2bus.org.NixosGeneration",
    "telemetry.d2bus.org.TelemetryService",
    "telemetry.d2bus.org.TelemetryBinding",
    "Credential",
    "Network",
    "Device",
    "usb.d2bus.org.UsbService",
    "usb.d2bus.org.UsbBinding",
    "security-key.d2bus.org.SecurityKeyService",
    "security-key.d2bus.org.SecurityKeyBinding",
    "display-wayland.d2bus.org.WaylandPolicy",
    "display-wayland.d2bus.org.WaylandSession",
    "audio.d2bus.org.AudioService",
    "audio.d2bus.org.AudioBinding",
    "shell-terminal.d2bus.org.ShellPool",
    "shell-terminal.d2bus.org.ShellSession",
    "Zone",
    "ZoneLink",
    "Provider",
    "Role",
    "RoleBinding",
    "Quota",
    "EmergencyPolicy",
    "ResourceExport",
    "ResourceImport",
    "Command",
    "Operation",
    "SeccompProfile",
];

/// One provider crate's declaration file.
#[derive(Deserialize)]
struct DeclarationFile {
    #[serde(rename = "crate")]
    crate_name: String,
    types: Vec<TypeDeclaration>,
}

/// One declared resource type row.
#[derive(Deserialize)]
struct TypeDeclaration {
    #[serde(rename = "resourceType")]
    resource_type: String,
}

/// The parsed per-crate type authority inputs.
struct AuthorityRegistry {
    /// Crate name -> declared type names.

    declarations: BTreeMap<String, BTreeSet<String>>,
    /// Crate name -> registered descriptor type names (extracted from the
    /// crate's Rust sources).
    descriptors: BTreeMap<String, BTreeSet<String>>,
}

/// Run the authority's gates: parity, drift, and regeneration idempotence,
/// over every artifact the authority emits: the Rust type authority const and
/// the Nix type registry and resource inventory.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn check(repo_root: &Path) -> Result<(), String> {
    let registry = load(repo_root)?;
    let errors = parity_errors(&registry);
    if !errors.is_empty() {
        return Err(format!(
            "resource-type-authority parity violations:\n- {}",
            errors.join("\n- ")
        ));
    }
    let artifacts = render_artifacts(repo_root, &registry)?;
    for (relative, rendered) in &artifacts {
        verify_committed(repo_root, relative, rendered)?;
    }
    let rendered_again = render_artifacts(repo_root, &registry)?;
    if rendered_again != artifacts {
        return Err("resource-type-authority regeneration is not idempotent".to_owned());
    }
    Ok(())
}

/// Regenerate every type-authority artifact from the declarations.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn regenerate(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let registry = load(repo_root)?;
    let errors = parity_errors(&registry);
    if !errors.is_empty() {
        return Err(format!(
            "refusing to regenerate the type authority while the parity gate fails:\n- {}",
            errors.join("\n- ")
        ));
    }
    let artifacts = render_artifacts(repo_root, &registry)?;
    let mut written = Vec::with_capacity(artifacts.len());
    for (relative, rendered) in artifacts {
        let artifact_path = repo_root.join(&relative);
        if let Some(parent) = artifact_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "cannot create the generated-artifact directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        fs::write(&artifact_path, &rendered).map_err(|error| {
            format!(
                "cannot write the generated artifact {}: {error}",
                artifact_path.display()
            )
        })?;
        written.push(artifact_path);
    }
    Ok(written)
}

/// Render every artifact the authority owns, in committed order: the Rust
/// authority const, the Nix standard type registry, and the Nix resource
/// inventory. The Nix registry and the inventory's `coreSchemaPointers` are
/// derived from the declared standard (unqualified) types; the inventory's
/// remaining tables are committed facts the shared renderer holds.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn render_artifacts(
    repo_root: &Path,
    registry: &AuthorityRegistry,
) -> Result<Vec<(String, String)>, String> {
    let standard = declared_standard_types(registry);
    let inventory = crate::nix_inventories::resource_inventories_module(repo_root, &standard)
        .map_err(|error| {
            format!("resource-type-authority inventory render failed: {error}")
        })?;
    Ok(vec![
        (GENERATED_ARTIFACT.to_owned(), render(registry)?),
        (NIX_RESOURCE_TYPES_OUT.to_owned(), render_nix_resource_types(&standard)),
        (NIX_RESOURCE_INVENTORIES_OUT.to_owned(), inventory),
    ])
}

/// Fail when the committed copy of one generated artifact differs from the
/// declarations' render.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn verify_committed(repo_root: &Path, relative: &str, rendered: &str) -> Result<(), String> {
    let artifact_path = repo_root.join(relative);
    let on_disk = fs::read_to_string(&artifact_path).map_err(|_| {
        format!(
            "resource-type-authority artifact is missing at {}; run `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        )
    })?;
    if on_disk != rendered {
        return Err(format!(
            "resource-type-authority drift: the committed generated artifact {} differs from the declarations' output; a hand edit or a stale generation must be repaired by `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        ));
    }
    Ok(())
}

/// The declared standard (unqualified) ResourceTypes in committed order, with
/// a declared standard type absent from the committed order appended after it
/// in sorted order. A qualified type (`<namespace>.d2bus.org.<Name>`) carries
/// a dot and never enters the standard registry.
fn declared_standard_types(registry: &AuthorityRegistry) -> Vec<String> {
    let mut declared = BTreeSet::new();
    for types in registry.declarations.values() {
        for name in types {
            if !name.contains('.') {
                declared.insert(name.clone());
            }
        }
    }
    let mut ordered = Vec::with_capacity(declared.len());
    for name in COMMITTED_NIX_STANDARD_ORDER {
        if let Some(type_name) = declared.take(*name) {
            ordered.push(type_name);
        }
    }
    ordered.extend(declared);
    ordered
}

/// A Nix double-quoted string.
fn nix_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str("\\$"),
            _ => out.push(character),
        }
    }
    out.push('"');
    out
}

/// Render the Nix standard type registry from the declared standard types.
///
/// The bytes are the committed `resource-types.nix` shape: the historical
/// registry header and the declared standard types in committed order.
fn render_nix_resource_types(standard: &[String]) -> String {
    let mut out = String::new();
    out.push_str("# Generated by `bazel run //packages/xtask:xtask -- gen-zone-nix-options`.\n");
    out.push_str("# Do not hand-edit: `make test-drift` compares this\n");
    out.push_str("# file byte-for-byte against the generator output.\n");
    out.push_str("#\n");
    out.push_str("# The canonical ADR 0046 standard ResourceType registry. Qualified\n");
    out.push_str("# Provider types are appended only from installed signed schemas and\n");
    out.push_str("# are therefore absent here.\n");
    out.push_str("# Provider-owned qualified types remain outside this registry: activation-nixos.d2bus.org.NixosGeneration, telemetry.d2bus.org.TelemetryBinding, telemetry.d2bus.org.TelemetryService.\n");
    out.push_str("[\n");
    for name in standard {
        out.push_str(&format!("  {}\n", nix_string(name)));
    }
    out.push_str("]\n");
    out
}

/// Load the declarations, then the registered descriptors from the tree.
fn load(repo_root: &Path) -> Result<AuthorityRegistry, String> {
    let declarations = load_declarations(repo_root)?;
    let descriptors = load_descriptors(repo_root, &declarations)?;
    Ok(AuthorityRegistry {
        declarations,
        descriptors,
    })
}

/// Read every provider crate's declaration file into a crate-keyed map.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_declarations(repo_root: &Path) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let packages_dir = repo_root.join(PACKAGES_DIR);
    let mut out = BTreeMap::new();
    let entries = fs::read_dir(&packages_dir).map_err(|error| {
        format!("cannot read {}: {error}", packages_dir.display())
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a packages entry: {error}"))?;
        let crate_name = entry.file_name().to_string_lossy().into_owned();
        if !crate_name.starts_with(PROVIDER_PREFIX) {

            continue;
        }
        let declaration_path = entry.path().join(DECLARATION_FILE);
        if !declaration_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&declaration_path).map_err(|error| {
            format!("cannot read the declaration {}: {error}", declaration_path.display())
        })?;
        let file: DeclarationFile = serde_json::from_str(&text).map_err(|error| {
            format!(
                "malformed declaration {}: {error}",
                declaration_path.display()
            )
        })?;
        if file.crate_name != crate_name {
            return Err(format!(
                "declaration-crate-mismatch: {} names crate \"{}\" but lives in crate \"{crate_name}\"",
                declaration_path.display(),
                file.crate_name
            ));
        }
        let types = file.types.into_iter().map(|t| t.resource_type).collect();
        out.insert(crate_name, types);
    }
    Ok(out)
}

/// Extract the type names each declaring crate's descriptor registers from
/// its Rust sources.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_descriptors(
    repo_root: &Path,
    crates: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let well_known = load_well_known(repo_root)?;
    let mut out = BTreeMap::new();
    for crate_name in crates.keys() {
        let src_dir = repo_root
            .join(PACKAGES_DIR)
            .join(crate_name)
            .join("src");
        let mut text = String::new();
        for path in collect_rs_files(&src_dir)? {
            text.push_str(
                &fs::read_to_string(&path)
                    .map_err(|error| {
                        format!("cannot read {}: {error}", path.display())
                    })?,
            );
            text.push('\n');
        }
        let mut names = BTreeSet::new();
        for ident in descriptor_type_idents(&text) {
            let name = well_known.get(&ident).ok_or_else(|| {
                format!(
                    "unknown-well-known-type: crate {crate_name}'s descriptor names WellKnownType::{ident}, which the well-known type vocabulary does not declare"
                )
            })?;
            names.insert(name.clone());
        }
        out.insert(crate_name.clone(), names);
    }
    Ok(out)
}

/// Read the `d2b-resource-types` `WellKnownType` vocabulary into an
/// ident -> type-name map.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_well_known(
    repo_root: &Path,
) -> Result<BTreeMap<String, String>, String> {
    let path = repo_root
        .join(PACKAGES_DIR)
        .join("d2b-resource-types/src/resource_type.rs");
    let text = fs::read_to_string(&path).map_err(|error| {
        format!("cannot read the well-known type vocabulary {}: {error}", path.display())
    })?;
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const ") else { continue };
        let Some((ident, rest)) = rest.split_once(": Self = Self(\"") else { continue };
        let Some(value) = rest.strip_suffix("\");") else { continue };
        map.insert(ident.to_owned(), value.to_owned());
    }
    Ok(map)
}

/// Recursively collect every `.rs` file under one source tree.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn collect_rs_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Err(format!(
            "missing-src: the declaring crate has no src tree at {}",
            dir.display()
        ));
    }
    let entries = fs::read_dir(dir).map_err(|error| {
        format!("cannot read {}: {error}", dir.display())
    })?;
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

/// Find every `WellKnownType::IDENT` a crate's descriptor registration
/// names: the `resource_type:` field initializer and the first argument of a
/// descriptor-builder call. Both are the shapes the existing driver crates
/// use to register their descriptors, so a drift there fails closed
/// rather than silently narrowing the registered set.
fn descriptor_type_idents(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    scan_descriptor_pattern(text, "resource_type:", &mut out);
    scan_descriptor_pattern(text, "descriptor(", &mut out);
    out
}

fn scan_descriptor_pattern(text: &str, marker: &str, out: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(relative) = rest.find(marker) {
        let after = &rest[relative + marker.len()..];
        let after = after.trim_start();
        if let Some(ident) = after.strip_prefix("WellKnownType::")
            && let Some((ident, _)) = ident.split_once(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        {
            out.insert(ident.to_owned());
        }
        rest = after;
    }
}

/// The declaration-to-descriptor parity violations: a declared type the
/// crate's descriptor does not register, a registered type the declaration
/// omits, and a type declared by two crates.
fn parity_errors(registry: &AuthorityRegistry) -> Vec<String> {
    let mut errors = Vec::new();
    let mut declared_by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (crate_name, types) in &registry.declarations {
        for type_name in types {
            declared_by.entry(type_name.as_str()).or_default().push(crate_name.as_str());
        }
    }
    for (type_name, crates) in &declared_by {
        if crates.len() > 1 {
errors.push(format!(
                "type-declared-twice: {type_name} is declared by both {}and {}",
                crates[0], crates[1]
            ));
        }
    }
    for (crate_name, declared) in &registry.declarations {



        let registered = registry
            .descriptors
            .get(crate_name)
            .cloned()
            .unwrap_or_default();
        for type_name in declared {
            if !registered.contains(type_name) {
                errors.push(format!(
                    "declared-but-not-registered: crate {crate_name} declares {type_name} but its registered descriptor does not"
                ));
            }
        }
        for type_name in &registered {
            if !declared.contains(type_name) {
                errors.push(format!(
                    "registered-but-not-declared: crate {crate_name}'s registered descriptor registers {type_name} but its declaration omits it"
                ));
            }
        }
    }
    errors
}

/// Emit the generated authority artifact text.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn render(registry: &AuthorityRegistry) -> Result<String, String> {
    let mut declared_all = BTreeSet::new();
    for types in registry.declarations.values() {
        declared_all.extend(types.iter().cloned());
    }
    let mut entries = Vec::with_capacity(declared_all.len());
    for name in COMMITTED_V3_ORDER {
        if let Some(type_name) = declared_all.take(*name) {
            entries.push(type_name);
        }
    }
    entries.extend(declared_all);
    let mut out = String::new();
out.push_str("// @generated\n");
    out.push_str("// Provenance:emitted from the per-crate `resource-types.json` declarations\n");
    out.push_str("// by `cargo xtask check-provider-crate-layout --fix`;the layout check's\n");
    out.push_str("// authority drift gate regenerates this file byte-for-byte,and refuses a\n");
    out.push_str("// hand edit.\n");
    out.push('\n');
    out.push_str("/// The resource types the v3 resource runtime owns end to end (R35/F1\n");
    out.push_str("/// exclusive per-type partition): served only by the per-zone manager plane.\n");
    out.push_str(&format!(
        "pub const V3_CONVERTED_RESOURCE_TYPES: [&str; {}] = [\n",
        entries.len()
    ));
    for entry in &entries {
        out.push_str("    \"");
        out.push_str(entry);
        out.push_str("\",\n");
    }
    out.push_str("];\n");
    Ok(out)
}



#[cfg(test)]
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
mod tests {
    use super::*;
    use std::fs;

    /// A throwaway fixture tree under the OS temp dir.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "d2b-u2-authority-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { root }
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, content).expect("write");
        }

        fn write_well_known(&self) {
            self.write(
                "packages/d2b-resource-types/src/resource_type.rs",
                "pub const ZONE: Self = Self(\"Zone\");\n\
                 pub const ROLE: Self = Self(\"Role\");\n\
                 pub const USB_SERVICE: Self = Self(\"usb.d2bus.org.UsbService\");\n\
                 pub const USB_BINDING: Self = Self(\"usb.d2bus.org.UsbBinding\");\n",
            );
        }

        /// Create the committed schema files the Nix inventory render requires
        /// for a fixture declaring the given standard types: one Core schema per
        /// standard type, the four semantic projection schemas,and the two provider
        /// farm schemas. The render only verifies existence, so empty files
        /// suffice.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_nix_inventory_schemas(&self, standard_types: &[&str]) {
            for resource_type in standard_types {
                self.write(
                    &format!(
                        "docs/reference/schemas/v3/core.d2bus.org_{resource_type}.schema.json"
                    ),
                    "",
                );
            }
            for file in [
                "audio.d2bus.org_projection_spec.schema.json",
                "security-key.d2bus.org_projection_spec.schema.json",
                "telemetry.d2bus.org_projection_spec.schema.json",
                "usb.d2bus.org_projection_spec.schema.json",
                "display-wayland.d2bus.org_WaylandPolicy.schema.json",
                "display-wayland.d2bus.org_WaylandSession.schema.json",
            ] {
                self.write(&format!("docs/reference/schemas/v3/{file}"), "");
            }
        }
    }

    impl Drop for Fixture {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn declaration_json(crate_name: &str, types: &[&str]) -> String {
        let types_json = types
            .iter()
            .map(|t| {
                format!(
                    "    {{\n      \"resourceType\": \"{t}\",\n      \"allowedSources\": [\"builtin\"],\n      \"verbs\": [],\n      \"execution\": [\"host\"],\n      \"exportable\": false,\n      \"reads\": []\n    }}"
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        format!(
            "{{\n  \"crate\": \"{crate_name}\",\n  \"types\": [\n{types_json}\n  ],\n  \"provides\": [],\n  \"roles\": [],\n  \"principals\": []\n}}\n"
        )
    }

    #[test]
    fn the_parity_check_fails_when_a_declaration_omits_a_registered_type() {
        let fixture = Fixture::new("omitted-type");
        fixture.write_well_known();
        fixture.write(
            "packages/d2b-provider-zone/resource-types.json",
            &declaration_json("d2b-provider-zone", &[]),
        );
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        let registry = load(&fixture.root).expect("load");
        let errors = parity_errors(&registry);
        assert!(
            errors.iter().any(|error| {
                error.contains("registered-but-not-declared")
                    && error.contains("d2b-provider-zone")
                    && error.contains("Zone")
            }),
            "expected the registered-but-not-declared violation naming both crate and type: {errors:?}"
        );
    }

    #[test]
    fn the_parity_check_fails_when_a_type_is_declared_twice() {
        let fixture = Fixture::new("duplicate-type");
        fixture.write_well_known();
        let zone_json = declaration_json("d2b-provider-zone", &["Zone"]);
        fixture.write("packages/d2b-provider-zone/resource-types.json", &zone_json);
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        let role_json = declaration_json("d2b-provider-role", &["Zone"]);
        fixture.write("packages/d2b-provider-role/resource-types.json", &role_json);
        fixture.write(
            "packages/d2b-provider-role/src/driver.rs",
            "pub fn role_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ROLE,\n    }\n}\n",
        );
        let registry = load(&fixture.root).expect("load");
        let errors = parity_errors(&registry);
        assert!(
            errors
                .iter()
                .any(|error| {
                    error.contains("type-declared-twice")
                        && error.contains("d2b-provider-zone")
                        && error.contains("d2b-provider-role")
                }),
            "expected the duplicate violation naming both crates: {errors:?}"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn the_drift_gate_fails_on_a_hand_edit_to_the_generated_artifact() {
        let fixture = Fixture::new("drift");
        fixture.write_well_known();
        fixture.write_nix_inventory_schemas(&["Zone"]);
        fixture.write("packages/d2b-provider-zone/resource-types.json", &declaration_json("d2b-provider-zone", &["Zone"]));
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        regenerate(&fixture.root).expect("regenerate");


        let artifact = fixture.root.join(GENERATED_ARTIFACT);
        let edited = format!("{}\n// hand edit\n", fs::read_to_string(&artifact).expect("read artifact"));
        fs::write(&artifact, edited).expect("hand edit");
        let error = check(&fixture.root).expect_err("a hand edit must fail the drift gate");
        assert!(
            error.contains("drift"),
            "expected a drift violation naming the gate: {error}"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn regeneration_is_idempotent() {
        let fixture = Fixture::new("idempotent");
        fixture.write_well_known();
        fixture.write_nix_inventory_schemas(&["Zone"]);
        fixture.write("packages/d2b-provider-zone/resource-types.json", &declaration_json("d2b-provider-zone", &["Zone"]));
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        let registry = load(&fixture.root).expect("load");
        let first = render(&registry).expect("render");
        let second = render(&registry).expect("render");
        assert_eq!(first, second, "rendering twice must yield identical bytes");
        regenerate(&fixture.root).expect("first regenerate");
        let after_first = fs::read_to_string(fixture.root.join(GENERATED_ARTIFACT)).expect("read");
        assert_eq!(after_first, first);
        regenerate(&fixture.root).expect("second regenerate");
        let after_second = fs::read_to_string(fixture.root.join(GENERATED_ARTIFACT)).expect("read");
        assert_eq!(after_first, after_second, "regenerating a generated artifact must be idempotent");
        check(&fixture.root).expect("the gate passes after regeneration");
    }

    #[test]
    fn the_policy_check_passes_on_a_matching_fixture() {
        let fixture = Fixture::new("happy");
        fixture.write_well_known();
        fixture.write_nix_inventory_schemas(&["Zone"]);
        fixture.write("packages/d2b-provider-zone/resource-types.json", &declaration_json("d2b-provider-zone", &["Zone"]));
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        regenerate(&fixture.root).expect("regenerate");
        check(&fixture.root).expect("the parity and drift gates pass on a matching tree");
    }

    /// The committed `resource-types.nix` bytes the authority's render must equal.
    const COMMITTED_RESOURCE_TYPES_NIX: &str = concat!(
        "# Generated by `bazel run //packages/xtask:xtask -- gen-zone-nix-options`.\n",
        "# Do not hand-edit: `make test-drift` compares this\n",
        "# file byte-for-byte against the generator output.\n",
        "#\n",
        "# The canonical ADR 0046 standard ResourceType registry. Qualified\n",
        "# Provider types are appended only from installed signed schemas and\n",
        "# are therefore absent here.\n",
        "# Provider-owned qualified types remain outside this registry: activation-nixos.d2bus.org.NixosGeneration, telemetry.d2bus.org.TelemetryBinding, telemetry.d2bus.org.TelemetryService.\n",
        "[\n",
        "  \"Zone\"\n",
        "  \"ZoneLink\"\n",
        "  \"Provider\"\n",
        "  \"Role\"\n",
        "  \"RoleBinding\"\n",
        "  \"Quota\"\n",
        "  \"EmergencyPolicy\"\n",
        "  \"Host\"\n",
        "  \"Guest\"\n",
        "  \"Process\"\n",
        "  \"EphemeralProcess\"\n",
        "  \"Volume\"\n",
        "  \"VolumeBinding\"\n",
        "  \"Network\"\n",
        "  \"Device\"\n",
        "  \"User\"\n",
        "  \"Credential\"\n",
        "  \"Endpoint\"\n",
        "  \"ResourceExport\"\n",
        "  \"ResourceImport\"\n",
        "  \"Command\"\n",
        "  \"Operation\"\n",
        "  \"SeccompProfile\"\n",
        "]\n",
    );

    #[test]
    fn the_nix_registry_is_derived_from_the_declared_standard_types() {
        let fixture = Fixture::new("nix-registry");
        fixture.write_well_known();
        let all = [
            "Zone",
            "ZoneLink",
            "Provider",
            "Role",
            "RoleBinding",
            "Quota",
            "EmergencyPolicy",
            "Host",
            "Guest",
            "Process",
            "EphemeralProcess",
            "Volume",
            "VolumeBinding",
            "Network",
            "Device",
            "User",
            "Credential",
            "Endpoint",
            "ResourceExport",
            "ResourceImport",
            "Command",
            "Operation",
            "SeccompProfile",
        ];
        fixture.write("packages/d2b-provider-zone/resource-types.json", &declaration_json("d2b-provider-zone", &all));
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        let registry = load(&fixture.root).expect("load");
        let standard = declared_standard_types(&registry);
        assert_eq!(
            standard,
            all.iter().map(|name| name.to_string()).collect::<Vec<_>>(),
            "the Nix registry is the declared standard set in committed order"
        );
        assert_eq!(
            render_nix_resource_types(&standard),
            COMMITTED_RESOURCE_TYPES_NIX,
            "the rendered Nix registry must match the committed bytes byte-for-byte"
        );
    }

    #[test]
    fn qualified_declared_types_do_not_enter_the_nix_registry_and_undocumented_types_append_sorted(
    ) {
        let declarations = BTreeMap::from([(
            "d2b-provider-faux".to_owned(),
            BTreeSet::from([
                "Zone".to_owned(),
                "ZoneLink".to_owned(),
                "usb.d2bus.org.UsbService".to_owned(),
                "Alpha".to_owned(),
                "Frobnicate".to_owned(),
            ]),
        )]);
        let registry = AuthorityRegistry {
            declarations,
            descriptors: BTreeMap::new(),
        };
        let standard = declared_standard_types(&registry);
        assert_eq!(
            standard,
            [
                "Zone".to_owned(),
                "ZoneLink".to_owned(),
                "Alpha".to_owned(),
                "Frobnicate".to_owned(),
            ],
            "qualified types must stay out and undocumented standard types append sorted"
        );
        let rendered = render_nix_resource_types(&standard);
        assert!(
            !rendered.contains("usb.d2bus.org.UsbService"),
            "a qualified type must not enter the standard registry: {rendered}"
        );
        assert!(rendered.contains("\"Alpha\"\n  \"Frobnicate\"\n"), "the appended block is sorted: {rendered}");
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_hand_edit_to_the_nix_registry_fails_the_drift_gate_and_regeneration_restores_it() {
        let fixture = Fixture::new("nix-fix");
        fixture.write_well_known();
        fixture.write_nix_inventory_schemas(&["Zone"]);
        fixture.write("packages/d2b-provider-zone/resource-types.json", &declaration_json("d2b-provider-zone", &["Zone"]));
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        regenerate(&fixture.root).expect("regenerate");
        let types_path = fixture.root.join("nixos-modules/generated/resource-types.nix");
        let before = fs::read_to_string(&types_path).expect("read");
        fs::write(&types_path, format!("{before}\n# hand edit\n")).expect("hand edit");
        let error = check(&fixture.root).expect_err("a hand edit must fail the drift gate");
        assert!(
            error.contains("drift"),
            "expected a drift violation naming the gate: {error}"
        );
        regenerate(&fixture.root).expect("regeneration restores the committed bytes");
        check(&fixture.root).expect("the gate passes after regeneration");
        assert_eq!(
            fs::read_to_string(&types_path).expect("read"),
            before,
            "regeneration must re-emit the committed bytes"
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn the_nix_inventory_is_also_gated_for_drift() {
        let fixture = Fixture::new("nix-inventory-drift");
        fixture.write_well_known();
        fixture.write_nix_inventory_schemas(&["Zone"]);
        fixture.write("packages/d2b-provider-zone/resource-types.json", &declaration_json("d2b-provider-zone", &["Zone"]));
        fixture.write(
            "packages/d2b-provider-zone/src/driver.rs",
            "pub fn zone_descriptor() -> d2b_resource_types::DriverDescriptor {\n    DriverDescriptor {\n        resource_type: WellKnownType::ZONE,\n    }\n}\n",
        );
        regenerate(&fixture.root).expect("regenerate");
        let inventory_path = fixture.root.join("nixos-modules/generated/resource-inventories.nix");
        fs::write(
            &inventory_path,
            format!("{}\n# hand edit\n", fs::read_to_string(&inventory_path).expect("read")),
        )
        .expect("hand edit");
        let error = check(&fixture.root).expect_err("a hand edit must fail the drift gate");
        assert!(
            error.contains("drift"),
            "expected a drift violation naming the gate: {error}"
        );
        regenerate(&fixture.root).expect("regeneration restores");
        check(&fixture.root).expect("the gate passes after regeneration");
    }
}