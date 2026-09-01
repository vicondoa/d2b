use std::collections::BTreeMap;

use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ResourceName, ResourceTypeName, ResourceUid, SchemaFingerprint, ZoneId,
};
use d2b_contracts_zone_session::v3::{
    generation_bundle::{BundleMetadata, BundleResource, ZoneBundle, ZoneBundleError},
    resource_bundle::ResourceBundle,
};

fn digest(byte: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn resource(resource_type: &str, name: &str, value: &str) -> BundleResource {
    BundleResource::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        BundleMetadata::new(
            ResourceName::parse(name).unwrap(),
            ZoneId::parse("work").unwrap(),
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap(),
        CanonicalJsonObject::parse(format!(r#"{{"value":"{value}"}}"#).as_bytes()).unwrap(),
    )
    .unwrap()
}

fn bundle() -> ZoneBundle {
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('a'),
        vec![resource("Network", "main", "configured")],
        BTreeMap::from([("network-local".to_owned(), digest('b'))]),
    )
    .unwrap()
}

#[test]
fn zone_uid_is_an_immutable_bundle_identity_anchor() {
    let zone_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("zone uid");
    let original = bundle().with_zone_uid(zone_uid.clone());
    let bytes = original.canonical_bytes().unwrap();
    let decoded = ZoneBundle::from_json(&bytes).unwrap();

    assert_eq!(decoded.zone_uid(), Some(&zone_uid));
    assert_eq!(decoded.canonical_bytes().unwrap(), bytes);

    let mut tampered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    tampered["zoneUid"] = serde_json::json!("223e4567-e89b-42d3-a456-426614174001");
    let decoded_tampered = ZoneBundle::from_json(&serde_json::to_vec(&tampered).unwrap())
        .expect("identity binding is checked by the Zone authority, not bundle parsing");
    assert_ne!(decoded_tampered.zone_uid(), Some(&zone_uid));
}

fn contains_key(value: &serde_json::Value, key: &str) -> bool {
    match value {
        serde_json::Value::Array(values) => values.iter().any(|value| contains_key(value, key)),
        serde_json::Value::Object(fields) => {
            fields.contains_key(key) || fields.values().any(|value| contains_key(value, key))
        }
        _ => false,
    }
}

#[test]
fn input_bundle_round_trips_with_stable_content_hash_and_provider_digests() {
    let original = bundle();
    let bytes = original.canonical_bytes().unwrap();
    let decoded = ZoneBundle::from_json(&bytes).unwrap();

    assert_eq!(decoded, original);
    assert_eq!(decoded.canonical_bytes().unwrap(), bytes);
    assert_eq!(
        decoded.content_hash(),
        &ZoneBundle::compute_content_hash(decoded.resources()).unwrap()
    );
    assert_eq!(decoded.provider_schema_digests().len(), 1);
    assert_eq!(
        decoded.provider_schema_digests().get("network-local"),
        Some(&digest('b'))
    );

    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.get("providerSchemaDigests").is_some());
}

#[test]
fn rendered_zone_resource_bundle_fixture_round_trips() {
    let Some(root) = std::env::var_os("D2B_FIXTURES") else {
        return;
    };
    let path = std::path::PathBuf::from(root)
        .join("zones")
        .join("local-root")
        .join("resource-bundle.json");
    let bytes = std::fs::read(&path).expect("rendered Zone resource bundle fixture");
    let bundle = ResourceBundle::from_json(bytes.trim_ascii_end())
        .expect("rendered Zone resource bundle contract");
    assert_eq!(bundle.zone.as_str(), "local-root");
    assert!(bundle.zone_uid().is_some());
    assert_eq!(
        ResourceBundle::from_json(&serde_json::to_vec(&bundle).unwrap()).unwrap(),
        bundle
    );
}

#[test]
fn bundle_resource_input_excludes_store_owned_metadata() {
    const _: [(); 4] = [(); BundleResource::INPUT_FIELD_NAMES.len()];

    assert_eq!(
        BundleResource::INPUT_FIELD_NAMES,
        ["apiVersion", "type", "metadata", "spec"]
    );
    let bytes = bundle().canonical_bytes().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let serialized_resource = &value["resources"][0];
    for forbidden in ["managedBy", "configurationGeneration"] {
        assert!(!contains_key(serialized_resource, forbidden));

        let mut direct = value.clone();
        direct["resources"][0][forbidden] = serde_json::json!("caller-owned");
        assert!(ZoneBundle::from_json(&serde_json::to_vec(&direct).unwrap()).is_err());

        let mut metadata = value.clone();
        metadata["resources"][0]["metadata"][forbidden] = serde_json::json!(1);
        assert!(ZoneBundle::from_json(&serde_json::to_vec(&metadata).unwrap()).is_err());
    }
}

#[test]
fn provider_schema_digest_verification_fails_closed() {
    let bundle = bundle();
    let exact = BTreeMap::from([("network-local".to_owned(), digest('b'))]);
    assert_eq!(bundle.verify_provider_schema_digests(&exact), Ok(()));

    let mismatched = BTreeMap::from([("network-local".to_owned(), digest('c'))]);
    assert_eq!(
        bundle.verify_provider_schema_digests(&mismatched),
        Err(ZoneBundleError::ProviderSchemaDigestMismatch)
    );
    assert_eq!(
        bundle.verify_provider_schema_digests(&BTreeMap::new()),
        Err(ZoneBundleError::ProviderSchemaDigestMismatch)
    );
}

#[test]
fn content_hash_changes_only_with_authored_resource_content() {
    let first = bundle();
    let changed_catalog = ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('d'),
        first.resources().to_vec(),
        BTreeMap::from([("network-local".to_owned(), digest('e'))]),
    )
    .unwrap();
    assert_eq!(first.content_hash(), changed_catalog.content_hash());

    let changed_resource = ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('a'),
        vec![resource("Network", "main", "changed")],
        BTreeMap::from([("network-local".to_owned(), digest('b'))]),
    )
    .unwrap();
    assert_ne!(first.content_hash(), changed_resource.content_hash());
}
