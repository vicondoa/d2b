//! Schema deny-unknown-fields + guest-control bounds policy lint, migrated from
//! `tests/static-invariant-deny-unknown-fields.sh`. The bash gate shelled out
//! to `nix-shell` + Python `jsonschema` to run full Draft 2020-12 validation
//! against committed fixtures; this Rust port reproduces the SAME invariants by
//! inspecting the committed schema JSON directly (a faithful structural
//! reduction, the same approach the nix-unit migrations took):
//!
//!   * the root object schema of every sensitive schema closes unknown fields
//!     (`additionalProperties: false`) — so an unknown top-level field is
//!     rejected, exactly what the bash gate's synthesize-instance +
//!     `__nixling_unknown_field__` + validate check proved;
//!   * the committed valid/invalid fixture pairs (bundle/host/closures) isolate
//!     an unknown field: the valid fixture's top-level keys are all declared
//!     root properties, and the invalid fixture carries a top-level key that is
//!     NOT a declared property (the unknown field the root's
//!     `additionalProperties: false` rejects);
//!   * EVERY object sub-schema in guest-control.json closes unknown fields
//!     (the bash gate's `assert_nested_unknowns_rejected`, which ran only for
//!     guest-control.json);
//!   * the guest-control string / chunk / terminal definition bounds have not
//!     drifted (`assert_guest_control_{string,chunk,terminal}_bounds`).
//!
//! This crate runs only from `tests/rust-workspace-checks.sh` against the real
//! checkout (excluded from the hermetic Nix sandbox build), so repo-file access
//! via the `nixling_contract_tests` helpers is sound.

use nixling_contract_tests::read_repo_file;
use serde_json::Value;

const SCHEMA_DIR: &str = "docs/reference/schemas/v2";
const FIXTURE_DIR: &str = "tests/fixtures/deny-unknown";

const SENSITIVE_SCHEMAS: &[&str] = &[
    "privileges",
    "processes",
    "minijail-profile",
    "bundle",
    "host",
    "closures",
    "guest-control",
];

/// Schemas with committed valid/invalid fixture pairs under `FIXTURE_DIR`.
const FIXTURE_STEMS: &[&str] = &["bundle", "host", "closures"];

fn load_schema(name: &str) -> Value {
    serde_json::from_str(&read_repo_file(&format!("{SCHEMA_DIR}/{name}.json")))
        .unwrap_or_else(|e| panic!("deny-unknown: {name}.json is not valid JSON: {e}"))
}

/// Resolve an internal `#/...` `$ref` chain (the only ref form these schemas
/// use), returning the pointed-at node. External refs are unsupported (and the
/// bash gate rejected them too).
fn resolve<'a>(mut node: &'a Value, root: &'a Value) -> &'a Value {
    while let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        let ptr = reference
            .strip_prefix("#/")
            .unwrap_or_else(|| panic!("deny-unknown: external $ref not supported: {reference}"));
        let mut cur = root;
        for part in ptr.split('/') {
            let key = part.replace("~1", "/").replace("~0", "~");
            cur = cur
                .get(&key)
                .unwrap_or_else(|| panic!("deny-unknown: dangling $ref {reference} at '{key}'"));
        }
        node = cur;
    }
    node
}

/// Whether a schema node is an object schema (`type: object` or carries
/// `properties`).
fn is_object_schema(s: &Value) -> bool {
    s.get("type").and_then(Value::as_str) == Some("object") || s.get("properties").is_some()
}

/// Whether an object schema closes unknown fields, either by
/// `additionalProperties: false` / `unevaluatedProperties: false`, or by being
/// an intentional open MAP (`additionalProperties` is itself a schema object).
fn closes_unknown_fields(s: &Value) -> bool {
    let ap = s.get("additionalProperties");
    if ap == Some(&Value::Bool(false))
        || s.get("unevaluatedProperties") == Some(&Value::Bool(false))
    {
        return true;
    }
    // A map type (additionalProperties is a schema) is intentionally open.
    matches!(ap, Some(Value::Object(_)))
}

/// Walk every distinct object sub-schema (root + definitions/$defs + nested
/// properties + items + oneOf/anyOf/allOf), invoking `visit` on each. Uses a
/// pointer-identity seen-set to terminate on recursive `$ref` cycles.
fn walk_object_schemas<'a>(
    node: &'a Value,
    root: &'a Value,
    path: String,
    seen: &mut Vec<*const Value>,
    visit: &mut dyn FnMut(&str, &Value),
) {
    let s = resolve(node, root);
    let id = s as *const Value;
    if seen.contains(&id) {
        return;
    }
    seen.push(id);

    if is_object_schema(s) {
        visit(&path, s);
    }
    let Some(map) = s.as_object() else { return };
    for key in ["definitions", "$defs", "properties"] {
        if let Some(Value::Object(children)) = map.get(key) {
            for (name, sub) in children {
                walk_object_schemas(sub, root, format!("{path}/{key}/{name}"), seen, visit);
            }
        }
    }
    if let Some(items) = map.get("items") {
        if items.is_object() {
            walk_object_schemas(items, root, format!("{path}/items"), seen, visit);
        }
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(subs)) = map.get(key) {
            for (i, sub) in subs.iter().enumerate() {
                walk_object_schemas(sub, root, format!("{path}/{key}/{i}"), seen, visit);
            }
        }
    }
}

#[test]
fn sensitive_schema_roots_close_unknown_fields() {
    for name in SENSITIVE_SCHEMAS {
        let schema = load_schema(name);
        assert!(
            is_object_schema(&schema),
            "deny-unknown: {name}.json root is not an object schema"
        );
        assert!(
            schema.get("additionalProperties") == Some(&Value::Bool(false))
                || schema.get("unevaluatedProperties") == Some(&Value::Bool(false)),
            "deny-unknown: {name}.json root must set additionalProperties:false \
             (an unknown top-level field would otherwise be accepted)"
        );
    }
}

#[test]
fn deny_unknown_fixtures_isolate_an_unknown_field() {
    for stem in FIXTURE_STEMS {
        let schema = load_schema(stem);
        let props: Vec<&str> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();

        let valid: Value =
            serde_json::from_str(&read_repo_file(&format!("{FIXTURE_DIR}/{stem}-valid.json")))
                .unwrap_or_else(|e| panic!("deny-unknown: {stem}-valid.json invalid JSON: {e}"));
        let invalid: Value = serde_json::from_str(&read_repo_file(&format!(
            "{FIXTURE_DIR}/{stem}-invalid.json"
        )))
        .unwrap_or_else(|e| panic!("deny-unknown: {stem}-invalid.json invalid JSON: {e}"));

        let valid_obj = valid
            .as_object()
            .unwrap_or_else(|| panic!("deny-unknown: {stem}-valid.json is not an object"));
        let invalid_obj = invalid
            .as_object()
            .unwrap_or_else(|| panic!("deny-unknown: {stem}-invalid.json is not an object"));

        // The valid fixture carries NO unknown top-level field (it validates).
        let valid_unknown: Vec<&String> = valid_obj
            .keys()
            .filter(|k| !props.contains(&k.as_str()))
            .collect();
        assert!(
            valid_unknown.is_empty(),
            "deny-unknown: {stem}-valid.json has unknown top-level field(s) {valid_unknown:?} \
             not declared in {stem}.json properties"
        );

        // The invalid fixture carries at least one unknown top-level field — the
        // field the root's additionalProperties:false rejects.
        let invalid_unknown: Vec<&String> = invalid_obj
            .keys()
            .filter(|k| !props.contains(&k.as_str()))
            .collect();
        assert!(
            !invalid_unknown.is_empty(),
            "deny-unknown: {stem}-invalid.json must carry an unknown top-level field \
             (it is the field schema validation must reject); declared props: {props:?}"
        );
    }
}

#[test]
fn guest_control_object_schemas_all_close_unknown_fields() {
    let schema = load_schema("guest-control");
    let mut seen: Vec<*const Value> = Vec::new();
    let mut open: Vec<String> = Vec::new();
    walk_object_schemas(
        &schema,
        &schema,
        "#".to_string(),
        &mut seen,
        &mut |path, s| {
            if !closes_unknown_fields(s) {
                open.push(path.to_string());
            }
        },
    );
    assert!(
        open.is_empty(),
        "deny-unknown: guest-control.json object schema(s) accept unknown fields \
         (missing additionalProperties:false): {open:?}"
    );
}

#[test]
fn guest_control_string_bounds_did_not_drift() {
    // (definition name, expected maxLength); minLength is always 1.
    const EXPECTED: &[(&str, u64)] = &[
        ("GuestSchemaVersion", 32),
        ("GuestConnectRequestLine", 64),
        ("GuestConnectAckLine", 64),
        ("GuestVmId", 128),
        ("RequestId", 128),
        ("ExecId", 128),
        ("GuestBootId", 128),
        ("CapabilitiesHash", 128),
        ("GuestArg", 4096),
        ("GuestUser", 128),
        ("GuestCwd", 4096),
        ("EnvKey", 128),
        ("EnvValue", 8192),
    ];
    let schema = load_schema("guest-control");
    let defs = schema
        .get("definitions")
        .and_then(Value::as_object)
        .expect("guest-control.json has definitions");
    for (name, max_length) in EXPECTED {
        let node = resolve(
            defs.get(*name)
                .unwrap_or_else(|| panic!("guest-control.json missing definition {name}")),
            &schema,
        );
        assert_eq!(
            node.get("type").and_then(Value::as_str),
            Some("string"),
            "{name} is not emitted as a string schema"
        );
        assert_eq!(
            node.get("minLength").and_then(Value::as_u64),
            Some(1),
            "{name} lost minLength=1"
        );
        assert_eq!(
            node.get("maxLength").and_then(Value::as_u64),
            Some(*max_length),
            "{name} lost maxLength={max_length}"
        );
    }
}

#[test]
fn guest_control_chunk_bounds_did_not_drift() {
    let schema = load_schema("guest-control");
    let prop = |definition: &str, field: &str| -> Value {
        let defs = schema
            .get("definitions")
            .and_then(Value::as_object)
            .unwrap();
        let node = resolve(
            defs.get(definition)
                .unwrap_or_else(|| panic!("missing definition {definition}")),
            &schema,
        );
        let p = node
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|m| m.get(field))
            .unwrap_or_else(|| panic!("missing {definition}.{field}"));
        resolve(p, &schema).clone()
    };

    // Byte-array data fields: minItems (where applicable), maxItems, and the
    // uint8 / 0..=255 byte item bounds.
    for (definition, min_items) in [
        ("WriteStdinRequest", Some(1u64)),
        ("ReadOutputResponse", None),
        ("ExecLogsResponse", None),
    ] {
        let p = prop(definition, "data");
        if let Some(mi) = min_items {
            assert_eq!(
                p.get("minItems").and_then(Value::as_u64),
                Some(mi),
                "{definition}.data lost minItems={mi}"
            );
        }
        assert_eq!(
            p.get("maxItems").and_then(Value::as_u64),
            Some(1_048_576),
            "{definition}.data lost maxItems=1048576"
        );
        let item = resolve(p.get("items").expect("data.items"), &schema);
        assert_eq!(
            item.get("format").and_then(Value::as_str),
            Some("uint8"),
            "{definition}.data byte item lost format=uint8"
        );
        assert_eq!(
            item.get("minimum").and_then(Value::as_f64),
            Some(0.0),
            "{definition}.data byte item lost minimum=0"
        );
        assert_eq!(
            item.get("maximum").and_then(Value::as_f64),
            Some(255.0),
            "{definition}.data byte item lost maximum=255"
        );
    }

    // maxLen integer fields: 1..=1048576.
    for definition in ["ReadOutputRequest", "ExecLogsRequest"] {
        let p = prop(definition, "maxLen");
        assert_eq!(
            p.get("minimum").and_then(Value::as_f64),
            Some(1.0),
            "{definition}.maxLen lost minimum=1"
        );
        assert_eq!(
            p.get("maximum").and_then(Value::as_f64),
            Some(1_048_576.0),
            "{definition}.maxLen lost maximum=1048576"
        );
    }
}

#[test]
fn guest_control_terminal_bounds_did_not_drift() {
    let schema = load_schema("guest-control");
    let defs = schema
        .get("definitions")
        .and_then(Value::as_object)
        .unwrap();
    for (definition, field) in [
        ("TerminalSize", "rows"),
        ("TerminalSize", "cols"),
        ("TtyWinResizeRequest", "rows"),
        ("TtyWinResizeRequest", "cols"),
    ] {
        let node = resolve(
            defs.get(definition)
                .unwrap_or_else(|| panic!("missing definition {definition}")),
            &schema,
        );
        let p = resolve(
            node.get("properties")
                .and_then(Value::as_object)
                .and_then(|m| m.get(field))
                .unwrap_or_else(|| panic!("missing {definition}.{field}")),
            &schema,
        );
        assert_eq!(
            p.get("minimum").and_then(Value::as_f64),
            Some(1.0),
            "{definition}.{field} terminal geometry lost minimum=1"
        );
        assert_eq!(
            p.get("maximum").and_then(Value::as_f64),
            Some(65535.0),
            "{definition}.{field} terminal geometry lost maximum=65535"
        );
    }
}
