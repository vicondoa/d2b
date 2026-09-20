//! The per-crate operation-row declarations and the generated broker catalog.
//!
//! Every `packages/d2b-provider-*/operations.json` declares the broker
//! operation rows its owning crate serves (KTD3/U2). This module:
//!
//! - runs the declaration-to-descriptor parity gate: a declared operation
//!   must be spelled in the crate's own sources (the descriptor's handler
//!   table), every operation reference the crate's descriptor registers
//!   must be declared, the declared family must be one of the crate's
//!   registered resource types (lowercased), and the declared provider must
//!   be the crate itself - a disagreement fails naming both;
//! - owns the drift gate over the generated catalog artifacts (the
//!   committed rows document and its five derived views): a hand edit fails
//!   and regeneration is idempotent.
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

use crate::gen_broker_operations;
use crate::resource_type_authority;

/// The directory-glob root the per-crate declarations live under.
const PACKAGES_DIR: &str = "packages";

/// Run the authority's gates: parity, drift, and regeneration idempotence,
/// over every artifact the broker-operation generator emits: the committed
/// rows document and the five derived views.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn check(repo_root: &Path) -> Result<(), String> {
    let errors = parity_errors(repo_root)?;
    if !errors.is_empty() {
        return Err(format!(
            "operation-row-authority parity violations:\n- {}",
            errors.join("\n- ")
        ));
    }
    let artifacts = gen_broker_operations::render_artifacts(repo_root)
        .map_err(|error| format!("operation-row generation failed: {error}"))?;
    for (relative, rendered) in &artifacts {
        verify_committed(repo_root, relative, rendered)?;
    }
    let rendered_again = gen_broker_operations::render_artifacts(repo_root)
        .map_err(|error| format!("operation-row generation failed: {error}"))?;
    if rendered_again != artifacts {
        return Err("operation-row regeneration is not idempotent".to_owned());
    }
    Ok(())
}

/// Regenerate every broker-operation artifact from the declarations and the
/// retained committed rows.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn regenerate(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let errors = parity_errors(repo_root)?;
    if !errors.is_empty() {
        return Err(format!(
            "refusing to regenerate the broker operation catalog while the parity gate fails:\n- {}",
            errors.join("\n- ")
        ));
    }
    let artifacts = gen_broker_operations::render_artifacts(repo_root)
        .map_err(|error| format!("operation-row generation failed: {error}"))?;
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

/// Fail when the committed copy of one generated artifact differs from the
/// declarations' render.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn verify_committed(repo_root: &Path, relative: &str, rendered: &str) -> Result<(), String> {
    let artifact_path = repo_root.join(relative);
    let on_disk = fs::read_to_string(&artifact_path).map_err(|_| {
        format!(
            "operation-row artifact is missing at {}; run `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        )
    })?;
    if on_disk != rendered {
        return Err(format!(
            "operation-row drift: the committed generated artifact {} differs from the declarations' output; a hand edit or a stale generation must be repaired by `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        ));
    }
    Ok(())
}

/// The declaration-to-descriptor parity violations: a declared operation the
/// crate's descriptor sources do not spell, an operation reference the
/// crate's descriptor registers that the declaration omits, a declared
/// family no registered descriptor type names, and a declared provider that
/// is not the crate itself.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn parity_errors(repo_root: &Path) -> Result<Vec<String>, String> {
    let declarations = gen_broker_operations::load_declarations(repo_root)
        .map_err(|error| format!("load operation declarations: {error}"))?;
    let mut errors = Vec::new();
    for (crate_name, rows) in &declarations {
        let src_dir = repo_root.join(PACKAGES_DIR).join(crate_name).join("src");
        let mut text = String::new();
        for path in collect_rs_files(&src_dir)? {
            text.push_str(
                &fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
            );
            text.push('\n');
        }
        let registered_types = resource_type_authority::descriptor_type_idents(&text);
        let families: BTreeSet<String> = registered_types
            .iter()
            .map(|ident| ident.to_ascii_lowercase())
            .collect();
        // The descriptor tables register operations by their lower-kebab
        // `Operation/<method>` reference (the method the session layer
        // addresses), while a declaration states the PascalCase wire
        // operation name; the parity surface is the declared method set.
        let consts = string_constants(&text);
        let registered_refs = descriptor_operation_refs(&text, &consts);
        let declared_methods: BTreeSet<String> = rows
            .iter()
            .map(|row| row.method.to_ascii_lowercase())
            .collect();
        for row in rows {
            // The declared operation must be spelled in the crate's own
            // sources: the descriptor's handler table is the execution
            // source, so a declaration the crate cannot serve is a parity
            // violation naming both.
            if !text.contains(&format!("\"{}\"", row.operation)) {
                errors.push(format!(
                    "declared-but-not-spelled: crate {crate_name} declares operation {} but its descriptor sources do not spell it",
                    row.operation
                ));
            }
            if !registered_refs.contains(&row.method.to_ascii_lowercase()) {
                errors.push(format!(
                    "declared-but-not-registered: crate {crate_name} declares operation {} with method {} but its descriptor registers no such operation reference",
                    row.operation, row.method
                ));
            }
            if row.declaring_provider != *crate_name {
                errors.push(format!(
                    "self-binding-mismatch: crate {crate_name} declares operation {} with declaring provider {}; a crate declares only its own rows",
                    row.operation, row.declaring_provider
                ));
            }
            if !families.contains(&row.family.to_ascii_lowercase()) {
                errors.push(format!(
                    "family-mismatch: crate {crate_name} declares operation {} with family {} which no registered descriptor type names",
                    row.operation, row.family
                ));
            }
        }
        // Every operation reference the crate's descriptor registers must be
        // declared: a served operation the declaration omits would be a
        // committed row no crate covers.
        for reference in &registered_refs {
            if !declared_methods.contains(reference) {
                errors.push(format!(
                    "registered-but-not-declared: crate {crate_name}'s descriptor registers operation {reference} but its declaration omits it"
                ));
            }
        }
    }
    Ok(errors)
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

/// Every `pub const NAME: &str = "VALUE";` in one crate's sources, as an
/// ident -> value map (the dynamic `format!("Operation/{CONST}")` refs the
/// descriptor tables build).
fn string_constants(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const ") else {
            continue;
        };
        let Some((ident, rest)) = rest.split_once(": &str = \"") else {
            continue;
        };
        let Some(value) = rest.strip_suffix("\";") else {
            continue;
        };
        map.insert(ident.to_owned(), value.to_owned());
    }
    map
}

/// The lowercase operation names a crate's descriptor tables register: every
/// literal `ResourceRef::parse("Operation/NAME")` reference and every
/// `format!("Operation/{CONST}")` reference resolved through the crate's
/// string constants.
fn descriptor_operation_refs(text: &str, consts: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let literal_marker = "ResourceRef::parse(\"Operation/";
    let mut rest = text;
    while let Some(relative) = rest.find(literal_marker) {
        let after = &rest[relative + literal_marker.len()..];
        if let Some((name, _)) = after.split_once('"') {
            out.insert(name.to_ascii_lowercase());
        }
        rest = after;
    }
    let format_marker = "format!(\"Operation/{";
    let mut rest = text;
    while let Some(relative) = rest.find(format_marker) {
        let after = &rest[relative + format_marker.len()..];
        if let Some((ident, _)) = after.split_once('}')
            && let Some(value) = consts.get(ident.trim())
        {
            out.insert(value.to_ascii_lowercase());
        }
        rest = after;
    }
    out
}

#[cfg(test)]
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
                "d2b-u2-operation-authority-{name}-{}-{nonce}",
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

        /// The committed rows document the generator reads as its retained
        /// input: one broker-generic handshake row.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_committed_catalog(&self) {
            self.write(
                "docs/reference/policy/broker-operations.json",
                "{\n\
                 \x20 \"version\": 1,\n\
                 \x20 \"rows\": [\n\
                 \x20   {\n\
                 \x20     \"operation\": \"Hello\",\n\
                 \x20     \"wireVariant\": \"Hello\",\n\
                 \x20     \"owner\": \"broker-generic\",\n\
                 \x20     \"family\": null,\n\
                 \x20     \"declaringProvider\": null,\n\
                 \x20     \"justification\": \"fixture handshake\",\n\
                 \x20     \"profiles\": [\"host\", \"guest\"],\n\
                 \x20     \"w3\": false,\n\
                 \x20     \"capabilities\": false,\n\
                 \x20     \"disposition\": \"callable-read-only\",\n\
                 \x20     \"dispositionTarget\": \"live read-only callable\",\n\
                 \x20     \"authz\": {\n\
                 \x20       \"subject\": \"handshake\",\n\
                 \x20       \"scope\": \"global\",\n\
                 \x20       \"allowedGroups\": [\"d2bd\"],\n\
                 \x20       \"destructive\": false,\n\
                 \x20       \"secretAccess\": \"None\",\n\
                 \x20       \"brokerRequired\": \"Yes\",\n\
                 \x20       \"auditMode\": \"Yes\"\n\
                 \x20     },\n\
                 \x20     \"audit\": {\n\
                 \x20       \"fields\": [\"Hello\"],\n\
                 \x20       \"required\": true,\n\
                 \x20       \"mode\": \"yes\"\n\
                 \x20     },\n\
                 \x20     \"payload\": {\n\
                 \x20       \"provenance\": \"wire\",\n\
                 \x20       \"schema\": null\n\
                 \x20     },\n\
                 \x20     \"deadline\": {\n\
                 \x20       \"tier\": \"standard\"\n\
                 \x20     }\n\
                 \x20   }\n\
                 \x20 ]\n\
                 }\n",
            );
        }

        /// A declaration file for one fixture crate with the given
        /// operations, each spelled in the fixture's descriptor source.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_declaration(&self, crate_name: &str, operations: &[&str]) {
            let rows = operations
                .iter()
                .map(|operation| {
                    format!(
                        "    {{\n      \"operation\": \"{operation}\",\n      \"family\": \"fixture\",\n      \"declaringProvider\": \"{crate_name}\",\n      \"service\": \"d2b.fixture\",\n      \"method\": \"{}\",\n      \"profiles\": [\"host\"],\n      \"w3\": false,\n      \"capabilities\": false,\n      \"disposition\": \"promoted-live\",\n      \"dispositionTarget\": \"live in production broker\",\n      \"authz\": {{\n        \"subject\": \"fixture\",\n        \"scope\": \"global\",\n        \"allowedGroups\": [\"d2bd\"],\n        \"destructive\": false,\n        \"secretAccess\": \"None\",\n        \"brokerRequired\": \"Yes\",\n        \"auditMode\": \"Yes\"\n      }},\n      \"audit\": {{\n        \"fields\": [\"{operation}\"],\n        \"required\": true,\n        \"mode\": \"yes\"\n      }},\n      \"payload\": {{\n        \"provenance\": \"wire\"\n      }},\n      \"deadline\": {{\n        \"tier\": \"standard\"\n      }}\n    }}",
                        operation.to_ascii_lowercase()
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n");
            self.write(
                &format!("packages/{crate_name}/operations.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"operations\": [\n{rows}\n  ]\n}}\n"
                ),
            );
        }

        /// The fixture crate's descriptor source spelling the operations'
        /// handler table: one wire-name constant and one
        /// `ResourceRef::parse("Operation/NAME")` per operation, plus the
        /// registered resource type.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_descriptor(&self, crate_name: &str, operations: &[&str]) {
            let consts = operations
                .iter()
                .map(|operation| {
                    format!(
                        "pub const {}_WIRE: &str = \"{operation}\";",
                        operation.to_ascii_uppercase()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let refs = operations
                .iter()
                .map(|operation| {
                    format!(
                        "            ResourceRef::parse(\"Operation/{}\"),",
                        operation.to_ascii_lowercase()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.write(
                &format!("packages/{crate_name}/src/driver.rs"),
                &format!(
                    "use d2b_resource_types::DriverDescriptor;\n\
                     {consts}\n\
                     pub fn fixture_descriptor() -> DriverDescriptor {{\n\
                     \x20   DriverDescriptor {{\n\
                     \x20       resource_type: WellKnownType::FIXTURE,\n\
                     \x20       operations: &[\n{refs}\n\x20       ],\n\
                     \x20   }}\n\
                     }}\n"
                ),
            );
        }
    }

    impl Drop for Fixture {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// A declaration that omits an operation its descriptor registers fails
    /// the parity check naming both the crate and the operation.
    #[test]
    fn the_parity_check_fails_when_a_declaration_omits_a_registered_operation() {
        let fixture = Fixture::new("omitted-operation");
        fixture.write_descriptor("d2b-provider-fixture", &["AlphaOp", "BetaOp"]);
        fixture.write_declaration("d2b-provider-fixture", &["AlphaOp"]);
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("registered-but-not-declared")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("betaop")
            }),
            "expected the registered-but-not-declared violation naming both crate and operation: {errors:?}"
        );
    }

    /// A declaration naming an operation the crate's descriptor sources do
    /// not spell fails the parity check naming both.
    #[test]
    fn the_parity_check_fails_when_a_declaration_names_an_unregistered_operation() {
        let fixture = Fixture::new("unregistered-operation");
        fixture.write_descriptor("d2b-provider-fixture", &["AlphaOp"]);
        fixture.write_declaration("d2b-provider-fixture", &["AlphaOp", "GhostOp"]);
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("declared-but-not-registered")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("GhostOp")
            }),
            "expected the declared-but-not-registered violation naming both: {errors:?}"
        );
    }

    /// A declaration whose declaring provider is not the crate itself fails
    /// the parity check.
    #[test]
    fn the_parity_check_fails_when_the_declaring_provider_is_not_the_crate() {
        let fixture = Fixture::new("foreign-provider");
        fixture.write_descriptor("d2b-provider-fixture", &["AlphaOp"]);
        fixture.write_declaration("d2b-provider-fixture", &["AlphaOp"]);
        let path = fixture.root.join("packages/d2b-provider-fixture/operations.json");
        let text = fs::read_to_string(&path).expect("declaration");
        fs::write(
            &path,
            text.replace(
                "\"declaringProvider\": \"d2b-provider-fixture\"",
                "\"declaringProvider\": \"d2b-provider-other\"",
            ),
        )
        .expect("mutate");
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("self-binding-mismatch")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("AlphaOp")
            }),
            "expected the self-binding violation naming both: {errors:?}"
        );
    }

    /// A declaration whose family no registered descriptor type names fails
    /// the parity check.
    #[test]
    fn the_parity_check_fails_when_the_family_disagrees_with_the_descriptor() {
        let fixture = Fixture::new("family-mismatch");
        fixture.write_descriptor("d2b-provider-fixture", &["AlphaOp"]);
        fixture.write_declaration("d2b-provider-fixture", &["AlphaOp"]);
        let path = fixture.root.join("packages/d2b-provider-fixture/operations.json");
        let text = fs::read_to_string(&path).expect("declaration");
        fs::write(
            &path,
            text.replace("\"family\": \"fixture\"", "\"family\": \"foreign\""),
        )
        .expect("mutate");
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("family-mismatch")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("AlphaOp")
            }),
            "expected the family violation naming both: {errors:?}"
        );
    }

    /// The parity check passes on a matching fixture, and the drift gate
    /// fails on a hand edit to a generated artifact.
    #[test]
    fn the_parity_and_drift_gates_pass_on_a_matching_fixture_and_fail_on_a_hand_edit() {
        let fixture = Fixture::new("happy");
        fixture.write_committed_catalog();
        fixture.write_descriptor("d2b-provider-fixture", &["AlphaOp"]);
        fixture.write_declaration("d2b-provider-fixture", &["AlphaOp"]);
        // The committed rows document and the five derived views: the
        // generator's own artifact list, written as the committed copies.
        let artifacts = gen_broker_operations::render_artifacts(&fixture.root)
            .expect("render the merged catalog");
        assert!(!artifacts.is_empty());
        for (relative, rendered) in &artifacts {
            fixture.write(relative, rendered);
        }
        check(&fixture.root).expect("the parity and drift gates pass on a matching tree");

        let first = artifacts
            .iter()
            .find(|(relative, _)| relative.ends_with(".rs"))
            .map(|(relative, _)| relative)
            .expect("a rust view");
        let artifact_path = fixture.root.join(first);
        let edited = format!(
            "{}\n// hand edit\n",
            fs::read_to_string(&artifact_path).expect("read artifact")
        );
        fs::write(&artifact_path, edited).expect("hand edit");
        let error = check(&fixture.root).expect_err("a hand edit must fail the drift gate");
        assert!(
            error.contains("drift"),
            "expected a drift violation naming the gate: {error}"
        );
    }
}