//! Device generation cleanup at the ZoneBundle boundary.
//!
//! The compiler/resource-compiler work is intentionally outside this test.
//! These cases pin the existing `ZoneBundle` input contract that the
//! configuration controller consumes: runtime ownership metadata is absent,
//! and cleanup identity is exactly `(type, name)`.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::{
    BundleMetadata, BundleResource, ZoneBundle, ZoneBundleError,
    v3::{CanonicalJsonObject, ResourceName, ResourceTypeName, SchemaFingerprint, ZoneId},
};

fn digest(byte: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

fn resource(
    resource_type: &str,
    name: &str,
    spec: &str,
    labels: BTreeMap<String, String>,
) -> BundleResource {
    BundleResource::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        BundleMetadata::new(
            ResourceName::parse(name).unwrap(),
            ZoneId::parse("work").unwrap(),
            None,
            labels,
            BTreeMap::new(),
        )
        .unwrap(),
        CanonicalJsonObject::parse(spec.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn bundle(resources: Vec<BundleResource>) -> ZoneBundle {
    ZoneBundle::build(
        ZoneId::parse("work").unwrap(),
        digest('a'),
        resources,
        BTreeMap::new(),
    )
    .unwrap()
}

fn identities(bundle: &ZoneBundle) -> BTreeSet<(String, String)> {
    bundle
        .resources()
        .iter()
        .map(|resource| {
            (
                resource.resource_type().as_str().to_owned(),
                resource.metadata().name().as_str().to_owned(),
            )
        })
        .collect()
}

#[test]
fn device_bundle_runtime_metadata_is_controller_owned() {
    let configured = bundle(vec![resource(
        "Device",
        "host-key",
        r#"{"providerRef":"Provider/device-security-key"}"#,
        BTreeMap::from([("purpose".to_owned(), "login".to_owned())]),
    )]);
    let rendered = String::from_utf8(configured.canonical_bytes().unwrap()).unwrap();

    assert!(rendered.contains(r#""type":"Device""#));
    assert!(!rendered.contains("managedBy"));
    assert!(!rendered.contains("configurationGeneration"));
    assert!(!rendered.contains("deletionRequestedAt"));
    assert!(!rendered.contains("status"));
}

#[test]
fn device_generation_cleanup_keys_by_type_and_name_only() {
    let prior = bundle(vec![
        resource(
            "Device",
            "kept",
            r#"{"providerRef":"Provider/device-security-key","mode":"old"}"#,
            BTreeMap::new(),
        ),
        resource(
            "Device",
            "removed",
            r#"{"providerRef":"Provider/device-security-key"}"#,
            BTreeMap::new(),
        ),
        resource(
            "Endpoint",
            "removed",
            r#"{"providerRef":"Provider/device-security-key"}"#,
            BTreeMap::new(),
        ),
    ]);
    let current = bundle(vec![resource(
        "Device",
        "kept",
        r#"{"providerRef":"Provider/device-security-key","mode":"new"}"#,
        BTreeMap::new(),
    )]);

    let removed = identities(&prior)
        .difference(&identities(&current))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(removed.contains(&("Device".to_owned(), "removed".to_owned())));
    assert!(removed.contains(&("Endpoint".to_owned(), "removed".to_owned())));
    assert!(!removed.contains(&("Device".to_owned(), "kept".to_owned())));
}

#[test]
fn device_bundle_rejects_runtime_management_fields() {
    let configured = bundle(vec![resource(
        "Device",
        "host-key",
        r#"{"providerRef":"Provider/device-security-key"}"#,
        BTreeMap::new(),
    )]);
    let mut value: serde_json::Value =
        serde_json::from_slice(&configured.canonical_bytes().unwrap()).unwrap();
    value["resources"][0]["metadata"]["managedBy"] = serde_json::json!("configuration");
    value["resources"][0]["metadata"]["configurationGeneration"] = serde_json::json!(7);

    assert_eq!(
        ZoneBundle::from_json(&serde_json::to_vec(&value).unwrap()),
        Err(ZoneBundleError::CanonicalJson)
    );
}
