//! The semantic Service and Binding schema artifact generator.
//!
//! `gen-semantic-service-schemas` writes the committed JSON Schema for each
//! base layer of the D098 common semantic catalog under
//! `docs/reference/schemas/v3/`. One artifact is written per frozen schema
//! identity: a `spec` and a `status` layer for each of the eight qualified
//! ResourceTypes, plus the strict projection schema of each family.
//!
//! The catalog in `d2b_contracts::v3::semantic_services` is the single source
//! of truth. Nothing here restates a field set, so a change to the frozen
//! bases cannot exist without moving these artifacts, which is what makes the
//! drift gate meaningful.
//!
//! The eight qualified types are installed from signed Provider schemas and
//! are deliberately not members of the standard ResourceType registry in
//! `zone_schema`, which is closed. This generator therefore emits schema
//! artifacts only and never touches that registry.

use std::{
    fs,
    path::{Path, PathBuf},
};

use d2b_contracts::v3::semantic_services::{
    SemanticLayerSchema, SemanticPairContract, SemanticTypeContract, catalog,
};
use serde_json::{Value, json};

/// The subdirectory the committed artifacts live in.
const OUT_DIR: &str = "docs/reference/schemas/v3";

/// Render one frozen base layer as a strict deny-unknown JSON Schema.
fn layer_schema(resource_type: &str, layer: &SemanticLayerSchema, description: &str) -> Value {
    let schema_id = layer.schema_id().to_canonical_string();
    let mut properties = serde_json::Map::new();
    for name in layer.allowed_names() {
        properties.insert(name.to_owned(), json!({}));
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("https://d2bus.org/schemas/v3/{schema_id}"),
        "title": schema_id,
        "description": description,
        "x-d2b-resource-type": resource_type,
        "x-d2b-schema-version": layer.version().to_canonical_string(),
        "x-d2b-schema-fingerprint": layer.fingerprint().as_str(),
        "type": "object",
        "additionalProperties": false,
        "required": layer.required_names().collect::<Vec<_>>(),
        "properties": Value::Object(properties),
    })
}

/// Render the strict projection schema of one family.
fn projection_schema(pair: &SemanticPairContract) -> Value {
    let projection = pair.projection();
    let factory = projection
        .projection_factory()
        .expect("every catalog projection has a constructible factory");
    let service_type = projection.service_type().to_canonical_string();
    let mut properties = serde_json::Map::new();
    for name in projection.projection_allowed_names() {
        properties.insert(name.to_owned(), json!({}));
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("https://d2bus.org/schemas/v3/{}/projection/spec", pair.family().namespace()),
        "title": format!("{}/projection/spec", pair.family().namespace()),
        "description": "Strict deny-unknown semantic base schema for a Core-generated projection Service. It carries only providerRef, the semantic base and import fields, and ResourceImport ownership; spec.provider is forbidden.",
        "x-d2b-resource-type": service_type,
        "x-d2b-binding-resource-type": projection.binding_type().to_canonical_string(),
        "x-d2b-projection-protocol-version": factory.projection_protocol_version().as_str(),
        "x-d2b-allowed-backing-ref-types": factory
            .allowed_backing_ref_types()
            .iter()
            .map(|name| name.to_canonical_string())
            .collect::<Vec<_>>(),
        "x-d2b-allowed-binding-target-ref-types": factory.allowed_binding_target_ref_types(),
        "x-d2b-exportability": factory.exportability(),
        "x-d2b-projection-schema-fingerprint": projection.projection_schema_fingerprint().as_str(),
        "x-d2b-factory-fingerprint": projection.factory_fingerprint().as_str(),
        "type": "object",
        "additionalProperties": false,
        "required": projection.projection_required_names().collect::<Vec<_>>(),
        "properties": Value::Object(properties),
    })
}

/// The committed file name for one schema identity.
fn artifact_name(schema_id: &str) -> String {
    format!("{}.schema.json", schema_id.replace('/', "_"))
}

fn write(out_dir: &Path, name: &str, value: &Value) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = out_dir.join(name);
    let mut data = serde_json::to_string_pretty(value)?;
    data.push('\n');
    fs::write(&path, data)?;
    Ok(path)
}

fn member_artifacts(
    out_dir: &Path,
    member: &SemanticTypeContract,
    written: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let resource_type = member.resource_type().to_canonical_string();
    for (layer, description) in [
        (
            member.spec(),
            "Frozen provider-neutral base spec layer. Every conformant implementation binds this exact version and fingerprint, accepts the canonical minimal base without spec.provider, and adds only its own strict extension schema.",
        ),
        (
            member.status(),
            "Frozen provider-neutral common status layer, written under status.resource. Implementation observation belongs only under status.provider and may not duplicate a field here.",
        ),
    ] {
        let name = artifact_name(layer.schema_id().as_str());
        written.push(write(
            out_dir,
            &name,
            &layer_schema(&resource_type, layer, description),
        )?);
    }
    Ok(())
}

/// Generate the committed schema artifacts for the semantic Service and
/// Binding bases.
pub fn gen_semantic_service_schemas(
    repo_root: &Path,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let out_dir = repo_root.join(OUT_DIR);
    fs::create_dir_all(&out_dir)?;

    let mut written = Vec::new();
    for pair in catalog() {
        member_artifacts(&out_dir, pair.service(), &mut written)?;
        member_artifacts(&out_dir, pair.binding(), &mut written)?;
        let name = artifact_name(&format!("{}/projection/spec", pair.family().namespace()));
        written.push(write(&out_dir, &name, &projection_schema(pair))?);
    }

    if written.is_empty() {
        // An empty success would let a drift gate see zero generated files and
        // conclude the artifacts are current.
        return Err("gen-semantic-service-schemas produced no artifacts".into());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_layer_produces_a_distinct_artifact_name() {
        let mut names: Vec<String> = Vec::new();
        for pair in catalog() {
            for member in [pair.service(), pair.binding()] {
                names.push(artifact_name(member.spec().schema_id().as_str()));
                names.push(artifact_name(member.status().schema_id().as_str()));
            }
            names.push(artifact_name(&format!(
                "{}/projection/spec",
                pair.family().namespace()
            )));
        }
        let unique: std::collections::BTreeSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        assert_eq!(names.len(), 20);
    }

    #[test]
    fn a_rendered_layer_is_strict_and_carries_its_frozen_identity() {
        let pair = catalog()[0];
        let rendered = layer_schema(
            &pair.service().resource_type().to_canonical_string(),
            pair.service().spec(),
            "test",
        );
        assert_eq!(rendered["additionalProperties"], json!(false));
        assert_eq!(rendered["x-d2b-schema-version"], json!("1.0"));
        assert!(
            rendered["x-d2b-schema-fingerprint"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert_eq!(
            rendered["x-d2b-resource-type"],
            json!("audio.d2bus.org.AudioService")
        );
    }

    #[test]
    fn a_rendered_projection_forbids_a_provider_extension_field() {
        for pair in catalog() {
            let rendered = projection_schema(pair);
            assert_eq!(rendered["additionalProperties"], json!(false));
            assert!(rendered["properties"].get("provider").is_none());
        }
    }

    #[test]
    fn every_declared_factory_field_is_published_by_the_generator() {
        for pair in catalog() {
            let rendered = projection_schema(pair);
            let factory = pair.projection().projection_factory().unwrap();
            let expected = [
                (
                    "x-d2b-resource-type",
                    json!(factory.service_type().to_canonical_string()),
                ),
                (
                    "x-d2b-binding-resource-type",
                    json!(factory.binding_type().to_canonical_string()),
                ),
                (
                    "x-d2b-projection-protocol-version",
                    json!(factory.projection_protocol_version().as_str()),
                ),
                (
                    "x-d2b-allowed-backing-ref-types",
                    json!(
                        factory
                            .allowed_backing_ref_types()
                            .iter()
                            .map(|name| name.to_canonical_string())
                            .collect::<Vec<_>>()
                    ),
                ),
                (
                    "x-d2b-allowed-binding-target-ref-types",
                    serde_json::to_value(factory.allowed_binding_target_ref_types()).unwrap(),
                ),
                (
                    "x-d2b-exportability",
                    serde_json::to_value(factory.exportability()).unwrap(),
                ),
                (
                    "x-d2b-projection-schema-fingerprint",
                    json!(factory.projection_schema_fingerprint().as_str()),
                ),
                (
                    "x-d2b-factory-fingerprint",
                    json!(factory.factory_fingerprint().as_str()),
                ),
            ];
            for (key, value) in expected {
                assert_eq!(rendered[key], value, "generator drifted for {key}");
            }
        }
    }

    #[test]
    fn an_artifact_missing_exportability_is_rejected_by_the_completeness_control() {
        let mut rendered = projection_schema(catalog()[0]);
        rendered
            .as_object_mut()
            .unwrap()
            .remove("x-d2b-exportability");
        let required = [
            "x-d2b-resource-type",
            "x-d2b-binding-resource-type",
            "x-d2b-projection-protocol-version",
            "x-d2b-allowed-backing-ref-types",
            "x-d2b-allowed-binding-target-ref-types",
            "x-d2b-exportability",
            "x-d2b-projection-schema-fingerprint",
            "x-d2b-factory-fingerprint",
        ];
        assert!(required.iter().any(|key| rendered.get(*key).is_none()));
    }
}
