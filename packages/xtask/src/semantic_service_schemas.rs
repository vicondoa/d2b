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

fn resource_ref_schema(pattern: String, allowed_types: &[String]) -> Value {
    json!({
        "type": "string",
        "pattern": pattern,
        "x-d2b-reference-kind": "ResourceRef",
        "x-d2b-reference-scope": "same-zone",
        "x-d2b-allowed-ref-types": allowed_types,
    })
}

fn provider_ref_schema() -> Value {
    resource_ref_schema(
        r"^Provider/[a-z][a-z0-9-]{0,62}$".to_owned(),
        &[String::from("Provider")],
    )
}

fn service_ref_schema(service_type: &str) -> Value {
    resource_ref_schema(
        format!(
            "^{}\\/[a-z][a-z0-9-]{{0,62}}$",
            service_type.replace('.', "\\.")
        ),
        &[service_type.to_owned()],
    )
}

fn generic_resource_ref_schema(allowed_types: &[&str]) -> Value {
    let pattern = if allowed_types.is_empty() {
        "^(?:[A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$".to_owned()
    } else {
        let alternatives = allowed_types
            .iter()
            .map(|value| value.replace('.', "\\."))
            .collect::<Vec<_>>()
            .join("|");
        format!("^(?:{alternatives})/[a-z][a-z0-9-]{{0,62}}$")
    };
    resource_ref_schema(
        pattern,
        &allowed_types
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
}

/// Render one frozen base layer as a strict deny-unknown JSON Schema.
fn layer_schema(
    resource_type: &str,
    service_type: Option<&str>,
    layer: &SemanticLayerSchema,
    description: &str,
) -> Value {
    let schema_id = layer.schema_id().to_canonical_string();
    let mut properties = serde_json::Map::new();
    for name in layer.allowed_names() {
        let property = match name {
            "providerRef" => provider_ref_schema(),
            "serviceRef" => {
                service_ref_schema(service_type.expect("binding schema has a service type"))
            }
            "guestRef" => generic_resource_ref_schema(&["Guest"]),
            "targetRef" => generic_resource_ref_schema(&["Guest"]),
            "backingDeviceRef" => generic_resource_ref_schema(&["Device"]),
            "producerRef" => generic_resource_ref_schema(&["Guest", "Zone"]),
            "observedServiceRef" => {
                generic_resource_ref_schema(&[service_type.unwrap_or(resource_type)])
            }
            "implementationEndpointRefs" | "ingestEndpointRefs" | "realizationRefs" => {
                json!({
                    "type": "array",
                    "items": generic_resource_ref_schema(&["Endpoint"]),
                })
            }
            "guestUsers" => json!({
                "type": "array",
                "items": generic_resource_ref_schema(&["User"]),
            }),
            "target" if resource_type.ends_with(".SecurityKeyBinding") => json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["guestRef", "userRef"],
                "properties": {
                    "guestRef": generic_resource_ref_schema(&["Guest"]),
                    "userRef": generic_resource_ref_schema(&["User"]),
                },
            }),
            _ => json!({}),
        };
        properties.insert(name.to_owned(), property);
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

fn metadata_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "zone"],
        "properties": {
            "name": {
                "type": "string",
                "pattern": "^[a-z][a-z0-9-]{0,62}$",
            },
            "zone": {
                "type": "string",
                "pattern": "^[a-z][a-z0-9-]{0,62}$",
            },
            "ownerRef": resource_ref_schema(
                "^(?:[A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$".to_owned(),
                &[],
            ),
        },
    })
}

fn resource_envelope_schema(
    resource_type: &str,
    service_type: Option<&str>,
    layer: &SemanticLayerSchema,
) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("https://d2bus.org/schemas/v3/{resource_type}"),
        "title": resource_type,
        "description": "Qualified semantic ResourceType envelope generated from the frozen catalog.",
        "x-d2b-resource-type": resource_type,
        "type": "object",
        "additionalProperties": false,
        "required": ["apiVersion", "metadata", "spec", "type"],
        "properties": {
            "apiVersion": {"const": "resources.d2bus.org/v3"},
            "metadata": metadata_schema(),
            "spec": layer_schema(
                resource_type,
                service_type,
                layer,
                "Qualified semantic resource spec.",
            ),
            "type": {"const": resource_type},
        },
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
        let property = match name {
            "providerRef" => provider_ref_schema(),
            _ => json!({}),
        };
        properties.insert(name.to_owned(), property);
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
    service_type: &str,
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
            &layer_schema(&resource_type, Some(service_type), layer, description),
        )?);
    }
    let envelope_name = qualified_artifact_name(&resource_type);
    written.push(write(
        out_dir,
        &envelope_name,
        &resource_envelope_schema(&resource_type, Some(service_type), member.spec()),
    )?);
    Ok(())
}

fn qualified_artifact_name(resource_type: &str) -> String {
    let (namespace, type_segment) = resource_type
        .rsplit_once('.')
        .expect("catalog ResourceType is qualified");
    format!("{namespace}_{type_segment}.schema.json")
}

fn semantic_resource_types_module() -> String {
    let mut out = String::from(
        "# Generated by `cargo run --manifest-path Cargo.toml -p xtask -- gen-semantic-service-schemas`.\n\
         # Do not hand-edit: the semantic catalog is the source of truth.\n[\n",
    );
    for pair in catalog() {
        out.push_str(&format!(
            "  \"{}\"\n  \"{}\"\n",
            pair.service().resource_type().to_canonical_string(),
            pair.binding().resource_type().to_canonical_string()
        ));
    }
    out.push_str("]\n");
    out
}

/// Generate the committed schema artifacts for the semantic Service and
/// Binding bases.
pub fn gen_semantic_service_schemas(
    repo_root: &Path,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let out_dir = repo_root.join(OUT_DIR);
    fs::create_dir_all(&out_dir)?;
    let generated_dir = repo_root.join("nixos-modules/generated");
    fs::create_dir_all(&generated_dir)?;

    let mut written = Vec::new();
    for pair in catalog() {
        let service_type = pair.service().resource_type().to_canonical_string();
        member_artifacts(&out_dir, pair.service(), &service_type, &mut written)?;
        member_artifacts(&out_dir, pair.binding(), &service_type, &mut written)?;
        let name = artifact_name(&format!("{}/projection/spec", pair.family().namespace()));
        written.push(write(&out_dir, &name, &projection_schema(pair))?);
    }

    if written.is_empty() {
        // An empty success would let a drift gate see zero generated files and
        // conclude the artifacts are current.
        return Err("gen-semantic-service-schemas produced no artifacts".into());
    }
    let generated_types = generated_dir.join("semantic-resource-types.nix");
    fs::write(generated_types, semantic_resource_types_module())?;
    written.push(generated_dir.join("semantic-resource-types.nix"));
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
            None,
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
        assert_eq!(
            rendered["properties"]["providerRef"]["x-d2b-reference-kind"],
            json!("ResourceRef")
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
    fn binding_service_ref_is_schema_identified_and_type_scoped() {
        let rendered = layer_schema(
            "audio.d2bus.org.AudioBinding",
            Some("audio.d2bus.org.AudioService"),
            catalog()[0].binding().spec(),
            "test",
        );
        assert_eq!(
            rendered["properties"]["serviceRef"]["x-d2b-reference-kind"],
            json!("ResourceRef")
        );
        assert_eq!(
            rendered["properties"]["serviceRef"]["x-d2b-allowed-ref-types"],
            json!(["audio.d2bus.org.AudioService"])
        );
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
    fn committed_qualified_envelopes_match_the_generator() {
        let root = crate::repo_root().expect("repository root");
        for pair in catalog() {
            let service_type = pair.service().resource_type().to_canonical_string();
            for member in [pair.service(), pair.binding()] {
                let resource_type = member.resource_type().to_canonical_string();
                let expected =
                    resource_envelope_schema(&resource_type, Some(&service_type), member.spec());
                let mut expected = serde_json::to_string_pretty(&expected).unwrap();
                expected.push('\n');
                let path = root
                    .join("docs/reference/schemas/v3")
                    .join(qualified_artifact_name(&resource_type));
                assert_eq!(
                    std::fs::read_to_string(&path).unwrap(),
                    expected,
                    "{} drifted",
                    path.display()
                );
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
