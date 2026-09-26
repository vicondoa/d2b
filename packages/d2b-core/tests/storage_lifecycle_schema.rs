//! The published storage lifecycle schema must accept the bytes the daemon
//! actually writes.
//!
//! The schema is generated from the same types `serde` serializes, so it can
//! only disagree with the wire where the two derive from different attribute
//! sets. `StorageLifecycleIssue` names its variant fields with serde's
//! container-level `rename_all_fields`, which schemars 0.8 does not read, so
//! the committed schema once required snake_case keys the daemon never wrote.
//! A drift gate cannot see that: the committed file equalled fresh generator
//! output. This test validates the serialized bytes against the committed
//! schema, so the two are compared to each other rather than to the generator.

use d2b_core::storage_lifecycle::{
    StorageContractValidationReason, StorageLifecycleIssue, StorageLifecycleReport,
    SyncContractValidationReason,
};
use serde_json::Value;

/// The committed consumer-facing schema, not a fresh generator run.
const PUBLISHED_SCHEMA: &str =
    include_str!("../../../docs/reference/schemas/v2/storage-lifecycle-report.json");

/// Keywords this validator interprets. A schema that grows a keyword outside
/// `INTERPRETED` and `NON_ASSERTING` fails the test instead of passing
/// unvalidated.
const INTERPRETED: [&str; 8] = [
    "$ref",
    "enum",
    "items",
    "minimum",
    "oneOf",
    "properties",
    "required",
    "type",
];

/// Keywords that assert nothing about the instance: annotation text, and the
/// definition table a `$ref` looks up.
const NON_ASSERTING: [&str; 5] = ["$schema", "definitions", "description", "format", "title"];

#[test]
fn every_issue_variant_satisfies_the_published_schema() {
    let schema: Value =
        serde_json::from_str(PUBLISHED_SCHEMA).expect("published schema parses");
    let report = StorageLifecycleReport {
        schema_version: "v2".to_owned(),
        storage_contract_present: true,
        sync_contract_present: true,
        path_count: 2,
        restart_policy_count: 1,
        lock_count: 1,
        issues: vec![
            StorageLifecycleIssue::MissingStorageContract,
            StorageLifecycleIssue::MissingSyncContract,
            StorageLifecycleIssue::LegacyBundleContractsUnavailable { bundle_version: 5 },
            StorageLifecycleIssue::BundleResolverUnavailable,
            StorageLifecycleIssue::StorageContractInvalid {
                contract_id: "storage.json".to_owned(),
                reason: StorageContractValidationReason::DuplicateStoragePathId,
                offending_id: Some("path:run-root".to_owned()),
            },
            StorageLifecycleIssue::SyncContractInvalid {
                contract_id: "sync.json".to_owned(),
                reason: SyncContractValidationReason::OfdLockMissingCloexec,
                offending_id: None,
            },
            StorageLifecycleIssue::MissingRestartPolicy {
                vm: "corp-vm".to_owned(),
                role_id: "cloud-hypervisor".to_owned(),
            },
            StorageLifecycleIssue::AdoptableMissingCgroupLeaf {
                vm: "corp-vm".to_owned(),
                role_id: "vhost-device-sound".to_owned(),
            },
        ],
    };
    let bytes = serde_json::to_value(&report).expect("report serializes");

    let variants = report
        .issues
        .iter()
        .map(StorageLifecycleIssue::kind_name)
        .collect::<Vec<_>>();
    assert_eq!(
        variants.len(),
        8,
        "one serialized issue per declared variant, so a new variant cannot skip the schema"
    );

    validate(&schema, &schema, &bytes).expect("serialized report satisfies the published schema");
}

/// Validate `instance` against `schema`, both borrowed from the published
/// document, returning the first disagreement.
fn validate(root: &Value, schema: &Value, instance: &Value) -> Result<(), String> {
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{instance} is checked against a non-object schema"))?;
    for keyword in object.keys() {
        if !INTERPRETED.contains(&keyword.as_str()) && !NON_ASSERTING.contains(&keyword.as_str()) {
            return Err(format!("{instance} is checked against unimplemented {keyword}"));
        }
    }
    if let Some(declared) = object.get("type")
        && !matches_type(declared, instance)
    {
        return Err(format!("{instance} is not of type {declared}"));
    }
    if let Some(allowed) = object.get("enum").and_then(Value::as_array)
        && !allowed.contains(instance)
    {
        return Err(format!("{instance} is outside the declared enum {allowed:?}"));
    }
    if let Some(floor) = object.get("minimum").and_then(Value::as_f64)
        && instance.as_f64().is_some_and(|number| number < floor)
    {
        return Err(format!("{instance} is below the declared minimum {floor}"));
    }
    if let Some(definition) = object.get("$ref").and_then(Value::as_str) {
        return validate(root, resolve(root, definition)?, instance);
    }
    if let Some(branches) = object.get("oneOf").and_then(Value::as_array) {
        let matched = branches
            .iter()
            .filter(|branch| validate(root, branch, instance).is_ok())
            .count();
        if matched != 1 {
            return Err(format!("{instance} satisfies {matched} oneOf branches, not one"));
        }
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object)
        && let Some(members) = instance.as_object()
    {
        for required in object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !members.contains_key(required) {
                return Err(format!("{instance} is missing the required key {required}"));
            }
        }
        for (key, value) in members {
            if let Some(property) = properties.get(key) {
                validate(root, property, value).map_err(|error| format!("{error} at {key}"))?;
            }
        }
    }
    if let Some(item) = object.get("items")
        && let Some(elements) = instance.as_array()
    {
        for (index, element) in elements.iter().enumerate() {
            validate(root, item, element).map_err(|error| format!("{error} at {index}"))?;
        }
    }
    Ok(())
}

/// A `type` keyword is either one name or a list of acceptable names.
fn matches_type(declared: &Value, instance: &Value) -> bool {
    match declared {
        Value::String(name) => matches_name(name, instance),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| matches_name(name, instance)),
        _ => false,
    }
}

fn matches_name(name: &str, instance: &Value) -> bool {
    match name {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "boolean" => instance.is_boolean(),
        "null" => instance.is_null(),
        "integer" => instance.is_i64() || instance.is_u64(),
        "number" => instance.is_number(),
        _ => false,
    }
}

/// Resolve a local `#/definitions/<name>` reference.
fn resolve<'a>(root: &'a Value, reference: &str) -> Result<&'a Value, String> {
    let name = reference
        .strip_prefix("#/definitions/")
        .ok_or_else(|| format!("{reference} is not a local definition reference"))?;
    root.get("definitions")
        .and_then(|definitions| definitions.get(name))
        .ok_or_else(|| format!("{reference} names no definition"))
}
