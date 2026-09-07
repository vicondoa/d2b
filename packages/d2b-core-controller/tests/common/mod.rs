#![allow(dead_code)]

use std::collections::BTreeMap;

use d2b_contracts_resource::v3::{
    CanonicalJsonObject, ResourceBundleGenerationId, ResourceName, ResourceTypeName,
    SchemaFingerprint, Timestamp, ZoneId,
};
use d2b_contracts_zone_session::v3::{BundleMetadata, BundleResource, ZoneBundle};
use d2b_core_controller::configuration::ResourceKey;

pub fn zone() -> ZoneId {
    ZoneId::parse("work").unwrap()
}

pub fn now() -> Timestamp {
    Timestamp::parse("2026-08-01T00:00:00.000Z").unwrap()
}

pub fn digest(byte: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

pub fn generation(byte: char) -> ResourceBundleGenerationId {
    ResourceBundleGenerationId::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
}

pub fn key(resource_type: &str, name: &str) -> ResourceKey {
    ResourceKey::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        ResourceName::parse(name).unwrap(),
    )
}

pub fn input(resource_type: &str, name: &str, value: &str) -> BundleResource {
    BundleResource::new(
        ResourceTypeName::parse(resource_type).unwrap(),
        BundleMetadata::new(
            ResourceName::parse(name).unwrap(),
            zone(),
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap(),
        CanonicalJsonObject::parse(format!(r#"{{"value":"{value}"}}"#).as_bytes()).unwrap(),
    )
    .unwrap()
}

pub fn bundle(byte: char, resources: impl IntoIterator<Item = BundleResource>) -> ZoneBundle {
    ZoneBundle::build(
        zone(),
        digest(byte),
        resources.into_iter().collect(),
        BTreeMap::new(),
    )
    .unwrap()
}
