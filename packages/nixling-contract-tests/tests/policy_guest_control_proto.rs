//! Guest-control protobuf policy lint, migrated from
//! `tests/guest-control-proto.sh`.

use std::collections::{BTreeMap, BTreeSet};

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;
use serde_json::{Map, Value};

const PROTO_REL: &str = "packages/nixling-contracts/proto/guest_control.proto";
const SCHEMA_REL: &str = "docs/reference/schemas/v2/guest-control.json";

#[derive(Clone, Debug)]
struct Field {
    name: String,
    proto_type: String,
    optional: bool,
    repeated: bool,
    number: u32,
}

#[derive(Debug)]
struct ProtoShape {
    blocks: BTreeMap<String, BTreeMap<String, String>>,
    message_fields: BTreeMap<String, Vec<Field>>,
    enum_values: BTreeMap<String, Vec<String>>,
}

fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

fn block_bodies(proto: &str, kind: &str) -> BTreeMap<String, String> {
    let pattern = Regex::new(&format!(r"(?m)^{kind}\s+(\w+)\s*\{{")).expect("valid block regex");
    let bytes = proto.as_bytes();
    let mut bodies = BTreeMap::new();

    for captures in pattern.captures_iter(proto) {
        let whole = captures.get(0).expect("block match");
        let name = captures.get(1).expect("block name").as_str().to_owned();
        let mut depth = 1_u32;
        let mut end = whole.end();
        while end < bytes.len() && depth > 0 {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        assert_eq!(
            depth, 0,
            "guest-control-proto: unterminated {kind} block {name}"
        );
        bodies.insert(name, proto[whole.end()..end - 1].to_owned());
    }

    bodies
}

fn parse_proto(proto: &str) -> ProtoShape {
    let field_re = Regex::new(r"^(optional\s+)?(repeated\s+)?([.\w]+)\s+(\w+)\s*=\s*(\d+)\s*;")
        .expect("valid field regex");
    let enum_re = Regex::new(r"^([A-Z0-9_]+)\s*=\s*\d+\s*;").expect("valid enum regex");

    let message_blocks = block_bodies(proto, "message");
    let enum_blocks = block_bodies(proto, "enum");

    let message_fields = message_blocks
        .iter()
        .map(|(name, body)| {
            let fields = body
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    let captures = field_re.captures(line)?;
                    Some(Field {
                        name: captures.get(4).expect("field name").as_str().to_owned(),
                        proto_type: captures.get(3).expect("field type").as_str().to_owned(),
                        optional: captures.get(1).is_some(),
                        repeated: captures.get(2).is_some(),
                        number: captures
                            .get(5)
                            .expect("field number")
                            .as_str()
                            .parse()
                            .expect("numeric field number"),
                    })
                })
                .collect();
            (name.clone(), fields)
        })
        .collect();

    let enum_values = enum_blocks
        .iter()
        .map(|(name, body)| {
            let values = body
                .lines()
                .filter_map(|line| {
                    enum_re
                        .captures(line.trim())
                        .map(|captures| captures.get(1).expect("enum value").as_str().to_owned())
                })
                .collect();
            (name.clone(), values)
        })
        .collect();

    ProtoShape {
        blocks: BTreeMap::from([
            ("message".to_owned(), message_blocks),
            ("enum".to_owned(), enum_blocks),
        ]),
        message_fields,
        enum_values,
    }
}

fn proto_shape() -> (String, ProtoShape) {
    assert!(
        repo_path_exists(PROTO_REL),
        "guest-control-proto: missing {PROTO_REL}"
    );
    let proto = read_repo_file(PROTO_REL);
    let shape = parse_proto(&proto);
    (proto, shape)
}

fn fields_by_name<'a>(shape: &'a ProtoShape, message: &str) -> BTreeMap<&'a str, &'a Field> {
    shape
        .message_fields
        .get(message)
        .unwrap_or_else(|| panic!("guest-control-proto: message {message} missing"))
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

fn assert_field(
    shape: &ProtoShape,
    proto_name: &str,
    field_name: &str,
    proto_type: &str,
    number: u32,
    optional: bool,
) {
    let fields = fields_by_name(shape, proto_name);
    let field = fields
        .get(field_name)
        .unwrap_or_else(|| panic!("guest-control-proto: {proto_name}.{field_name} missing"));
    assert_eq!(
        (field.proto_type.as_str(), field.number, field.optional,),
        (proto_type, number, optional),
        "guest-control-proto: {proto_name}.{field_name} drifted"
    );
}

fn camel(name: &str) -> String {
    let mut parts = name.split('_');
    let Some(head) = parts.next() else {
        return String::new();
    };
    let mut out = head.to_owned();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}

fn schema() -> Value {
    assert!(
        repo_path_exists(SCHEMA_REL),
        "guest-control-proto: missing generated schema {SCHEMA_REL}"
    );
    serde_json::from_str(&read_repo_file(SCHEMA_REL))
        .expect("guest-control-proto: generated schema parses as JSON")
}

fn definitions(schema: &Value) -> &Map<String, Value> {
    schema
        .get("definitions")
        .and_then(Value::as_object)
        .expect("guest-control-proto: schema definitions object missing")
}

fn schema_core(prop: &Value) -> Value {
    if let Some(any_of) = prop.get("anyOf").and_then(Value::as_array) {
        let variants = any_of
            .iter()
            .filter(|variant| variant.get("type").and_then(Value::as_str) != Some("null"))
            .collect::<Vec<_>>();
        if let [variant] = variants.as_slice() {
            return (*variant).clone();
        }
    }

    if let Some(types) = prop.get("type").and_then(Value::as_array) {
        let non_null = types
            .iter()
            .filter(|entry| entry.as_str() != Some("null"))
            .collect::<Vec<_>>();
        if let [only] = non_null.as_slice() {
            let mut copy = prop.clone();
            if let Some(object) = copy.as_object_mut() {
                object.insert("type".to_owned(), (*only).clone());
            }
            return copy;
        }
    }

    prop.clone()
}

fn ref_name(prop: &Value) -> Option<String> {
    prop.get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/definitions/"))
        .map(str::to_owned)
}

fn deref(prop: &Value, defs: &Map<String, Value>) -> Value {
    ref_name(prop)
        .and_then(|referenced| defs.get(&referenced).cloned())
        .unwrap_or_else(|| prop.clone())
}

fn message_map(proto_type: &str) -> &str {
    match proto_type {
        "RequestMetadata" => "GuestRequestMetadata",
        "ExecRequestMetadata" => "GuestExecRequestMetadata",
        _ => proto_type,
    }
}

fn scalar_type(proto_type: &str) -> Option<(&'static str, Option<&'static str>)> {
    match proto_type {
        "string" => Some(("string", None)),
        "bool" => Some(("boolean", None)),
        "uint32" => Some(("integer", Some("uint32"))),
        "uint64" => Some(("integer", Some("uint64"))),
        "int32" => Some(("integer", Some("int32"))),
        _ => None,
    }
}

fn number_matches(prop: &Value, key: &str, expected: f64) -> bool {
    prop.get(key)
        .and_then(Value::as_f64)
        .is_some_and(|actual| (actual - expected).abs() < f64::EPSILON)
}

fn assert_scalar_shape(proto_name: &str, field: &Field, prop: &Value, defs: &Map<String, Value>) {
    let core = schema_core(prop);
    let proto_type = field.proto_type.as_str();

    if field.repeated {
        assert_eq!(
            core.get("type").and_then(Value::as_str),
            Some("array"),
            "guest-control-proto: {proto_name}.{} is repeated in proto but not array in schema",
            field.name
        );
        assert_ne!(
            proto_type, "bytes",
            "guest-control-proto: {proto_name}.{} cannot be repeated bytes",
            field.name
        );
        let item = schema_core(core.get("items").unwrap_or(&Value::Null));
        let scalar_field = Field {
            repeated: false,
            ..field.clone()
        };
        assert_scalar_shape(proto_name, &scalar_field, &item, defs);
        return;
    }

    if proto_type == "bytes" {
        let core = deref(&core, defs);
        let item = schema_core(core.get("items").unwrap_or(&Value::Null));
        assert!(
            core.get("type").and_then(Value::as_str) == Some("array")
                && item.get("type").and_then(Value::as_str) == Some("integer")
                && item.get("format").and_then(Value::as_str) == Some("uint8")
                && number_matches(&item, "minimum", 0.0)
                && number_matches(&item, "maximum", 255.0),
            "guest-control-proto: {proto_name}.{} bytes field is not byte-array shaped in schema",
            field.name
        );
        return;
    }

    if let Some((expected_type, expected_format)) = scalar_type(proto_type) {
        let scalar_matches = core.get("type").and_then(Value::as_str) == Some(expected_type)
            && expected_format
                .is_none_or(|format| core.get("format").and_then(Value::as_str) == Some(format));
        if scalar_matches {
            return;
        }
        if proto_type == "string"
            && ref_name(&core)
                .and_then(|referenced| defs.get(&referenced))
                .map(schema_core)
                .and_then(|node| node.get("type").and_then(Value::as_str).map(str::to_owned))
                == Some("string".to_owned())
        {
            return;
        }
        panic!(
            "guest-control-proto: {proto_name}.{} type drift: proto {proto_type}, schema {core}",
            field.name
        );
    }

    let schema_ref = ref_name(&core);
    let expected_ref = message_map(proto_type);
    assert_eq!(
        schema_ref.as_deref(),
        Some(expected_ref),
        "guest-control-proto: {proto_name}.{} ref drift",
        field.name
    );
}

fn has_nullable_branch(prop: &Value) -> bool {
    let type_has_null = match prop.get("type") {
        Some(Value::String(typ)) => typ == "null",
        Some(Value::Array(types)) => types.iter().any(|typ| typ.as_str() == Some("null")),
        _ => false,
    };
    let any_of_has_null = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|variants| {
            variants
                .iter()
                .any(|variant| variant.get("type").and_then(Value::as_str) == Some("null"))
        });
    type_has_null || any_of_has_null
}

fn enum_prefix(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower_or_digit = false;
    for ch in name.chars() {
        if ch.is_uppercase() && prev_lower_or_digit {
            out.push('_');
        }
        out.extend(ch.to_uppercase());
        prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    out.push('_');
    out
}

fn enum_schema_values(values: &[String], enum_name: &str) -> BTreeSet<String> {
    let prefix = enum_prefix(enum_name);
    values
        .iter()
        .filter(|value| !value.ends_with("_UNSPECIFIED"))
        .map(|value| value.strip_prefix(&prefix).unwrap_or(value))
        .map(|value| value.to_ascii_lowercase().replace('_', "-"))
        .collect()
}

#[test]
fn guest_control_proto_declares_service_methods_and_descriptor_sentinels() {
    let (proto, shape) = proto_shape();

    assert!(
        any_line_matches(&proto, r"^\s*service\s+GuestControl\s*\{"),
        "guest-control-proto: descriptor missing GuestControl service"
    );
    for method in [
        "Hello",
        "Authenticate",
        "Capabilities",
        "Health",
        "ExecCreate",
        "ExecInspect",
        "ExecWait",
        "ExecLogs",
        "WriteStdin",
        "ReadOutput",
        "CloseStdin",
        "TtyWinResize",
        "ExecSignal",
        "ExecCancel",
        "ReadGuestFile",
        "UsbipImport",
        "UsbipStatus",
    ] {
        assert!(
            any_line_matches(&proto, &format!(r"^\s*rpc\s+{method}\s*\(")),
            "guest-control-proto: descriptor missing method {method}"
        );
    }

    let all_field_names = shape
        .message_fields
        .values()
        .flatten()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    for field in [
        "guest_boot_id",
        "pending_read_output_waits_per_stream",
        "pending_exec_waits_per_vm",
        "rpc_rate_per_connection_per_second",
        "rpc_rate_per_vm_burst",
        "end_offset",
        "timed_out",
        "retry_after_ms",
        "host_auth_tag",
        "guest_auth_tag",
        "bus_id",
        "detached_ports",
        "imports",
        "tcp_port",
    ] {
        assert!(
            all_field_names.contains(field),
            "guest-control-proto: descriptor missing field {field}"
        );
    }

    let optional_count = shape
        .message_fields
        .values()
        .flatten()
        .filter(|field| field.optional)
        .count();
    assert!(
        optional_count >= 6,
        "guest-control-proto: expected optional scalar/string fields in descriptor"
    );

    let terminal_status = shape
        .blocks
        .get("message")
        .and_then(|messages| messages.get("TerminalStatus"))
        .expect("guest-control-proto: TerminalStatus message missing");
    assert!(
        any_line_matches(terminal_status, r"^\s*oneof\s+outcome\s*\{"),
        "guest-control-proto: descriptor missing TerminalStatus outcome oneof"
    );

    let all_enum_values = shape
        .enum_values
        .values()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for value in [
        "WRITE_DISPOSITION_REJECTED",
        "GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH",
        "GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE",
        "GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_PATH_UNSAFE",
        "GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED",
        "GUEST_CONTROL_ERROR_KIND_CWD_INVALID",
        "GUEST_CONTROL_ERROR_KIND_CWD_DENIED",
        "GUEST_CAPABILITY_USBIP_IMPORT",
        "GUEST_CAPABILITY_USBIP_STATUS",
        "GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_FAILED",
        "GUEST_CONTROL_ERROR_KIND_USBIP_COMMAND_TIMEOUT",
        "GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_HOST",
        "GUEST_CONTROL_ERROR_KIND_USBIP_INVALID_OUTPUT",
        "USBIP_IMPORT_ACTION_ATTACH",
        "USBIP_IMPORT_ACTION_DETACH",
    ] {
        assert!(
            all_enum_values.contains(value),
            "guest-control-proto: descriptor missing enum value {value}"
        );
    }
    for rejected in [
        "GUEST_CAPABILITY_READ_GUEST_CONFIG",
        "SIGNAL_TARGET_ROOT_PROCESS",
    ] {
        assert!(
            !all_enum_values.contains(rejected),
            "guest-control-proto: descriptor unexpectedly contains {rejected}"
        );
    }
}

#[test]
fn guest_control_proto_field_numbers_and_reserved_slots_are_stable() {
    let (_proto, shape) = proto_shape();

    assert_field(&shape, "HelloRequest", "host_nonce", "bytes", 2, false);
    assert_field(&shape, "HelloResponse", "guest_nonce", "bytes", 1, false);
    assert_field(
        &shape,
        "AuthenticateRequest",
        "host_auth_tag",
        "bytes",
        6,
        false,
    );
    assert_field(
        &shape,
        "AuthenticateResponse",
        "guest_auth_tag",
        "bytes",
        1,
        true,
    );
    assert_field(
        &shape,
        "AuthenticateResponse",
        "capabilities_hash",
        "string",
        2,
        true,
    );
    assert_field(
        &shape,
        "AuthenticateResponse",
        "health",
        "HealthResponse",
        3,
        false,
    );
    assert_field(
        &shape,
        "AuthenticateResponse",
        "capabilities",
        "CapabilitiesResponse",
        4,
        false,
    );
    assert_field(
        &shape,
        "UsbipImportRequest",
        "action",
        "UsbipImportAction",
        2,
        false,
    );
    assert_field(&shape, "UsbipImportRequest", "host", "string", 3, false);
    assert_field(&shape, "UsbipImportRequest", "bus_id", "string", 4, false);
    assert_field(
        &shape,
        "UsbipImportResponse",
        "detached_ports",
        "uint32",
        3,
        false,
    );
    assert_field(&shape, "UsbipStatusRequest", "host", "string", 2, true);
    assert_field(&shape, "UsbipStatusRequest", "bus_id", "string", 3, true);
    assert_field(&shape, "UsbipStatusEntry", "port", "uint32", 1, false);
    assert_field(&shape, "UsbipStatusEntry", "host", "string", 2, false);
    assert_field(&shape, "UsbipStatusEntry", "tcp_port", "uint32", 3, false);
    assert_field(&shape, "UsbipStatusEntry", "bus_id", "string", 4, false);

    let hello_response_fields = fields_by_name(&shape, "HelloResponse");
    for rejected in ["health", "capabilities_hash"] {
        assert!(
            !hello_response_fields.contains_key(rejected),
            "guest-control-proto: HelloResponse must not expose pre-auth {rejected}"
        );
    }
    assert!(
        shape.message_fields.contains_key("AuthenticateRequest")
            && shape.message_fields.contains_key("AuthenticateResponse"),
        "guest-control-proto: Authenticate messages missing from proto"
    );
    let hello_response_body = shape
        .blocks
        .get("message")
        .and_then(|messages| messages.get("HelloResponse"))
        .expect("guest-control-proto: HelloResponse message missing");
    assert!(
        hello_response_body.contains("reserved 4, 5;")
            && hello_response_body.contains(r#"reserved "capabilities_hash", "health";"#),
        "guest-control-proto: HelloResponse must reserve retired pre-auth health/capability fields"
    );
}

#[test]
fn guest_control_proto_schema_camelcase_fields_limits_and_shapes_match() {
    let (_proto, shape) = proto_shape();
    let schema = schema();
    let defs = definitions(&schema);

    let limit_props = defs
        .get("GuestEffectiveLimits")
        .and_then(|node| node.get("properties"))
        .and_then(Value::as_object)
        .expect("guest-control-proto: GuestEffectiveLimits schema properties missing");
    for (field_name, expected_maximum) in [
        ("maxChunkBytes", 1_048_576.0),
        ("maxRecvMessageBytes", 4_194_304.0),
        ("decodedWriteStdinBytesPerConnection", 16_777_216.0),
        ("writeStdinHandlersPerConnection", 4.0),
        ("stdinQueueChunksPerExec", 1.0),
        ("stdoutLiveBufferBytes", 8_388_608.0),
        ("stderrLiveBufferBytes", 8_388_608.0),
        ("detachedStdoutLogBytes", 4_194_304.0),
        ("detachedStderrLogBytes", 4_194_304.0),
        ("longPollTimeoutMs", 1_000.0),
        ("slowConsumerGraceMs", 300_000.0),
        ("execSessionsPerVm", 256.0),
        ("attachedSessionsPerVm", 64.0),
        ("pendingReadOutputWaitsPerStream", 512.0),
        ("pendingExecWaitsPerVm", 512.0),
        ("rpcRatePerConnectionPerSecond", 200.0),
        ("rpcRatePerVmBurst", 1_000.0),
    ] {
        let prop = schema_core(limit_props.get(field_name).unwrap_or(&Value::Null));
        assert!(
            number_matches(&prop, "maximum", expected_maximum),
            "guest-control-proto: GuestEffectiveLimits.{field_name} missing maximum {expected_maximum}"
        );
    }
    for field_name in ["maxChunkBytes", "maxRecvMessageBytes"] {
        let prop = schema_core(limit_props.get(field_name).unwrap_or(&Value::Null));
        assert!(
            number_matches(&prop, "minimum", 1.0),
            "guest-control-proto: GuestEffectiveLimits.{field_name} must have minimum 1"
        );
    }

    for (proto_name, fields) in &shape.message_fields {
        if proto_name == "TerminalStatus" {
            continue;
        }
        let schema_name = message_map(proto_name);
        let node = defs.get(schema_name).unwrap_or_else(|| {
            panic!("guest-control-proto: schema missing definition for proto message {proto_name}")
        });
        let props = node
            .get("properties")
            .and_then(Value::as_object)
            .expect("guest-control-proto: schema message properties missing");
        let schema_props = props.keys().cloned().collect::<BTreeSet<_>>();
        let wanted = fields
            .iter()
            .map(|field| camel(&field.name))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            schema_props, wanted,
            "guest-control-proto: {proto_name}/{schema_name} field drift"
        );

        let required = node
            .get("required")
            .and_then(Value::as_array)
            .map(|required| {
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        for field in fields {
            let schema_field = camel(&field.name);
            let prop = props.get(&schema_field).unwrap_or_else(|| {
                panic!("guest-control-proto: {schema_name}.{schema_field} missing in schema")
            });
            assert_scalar_shape(proto_name, field, prop, defs);
            if field.optional {
                assert!(
                    !required.contains(schema_field.as_str()),
                    "guest-control-proto: {schema_name}.{schema_field} is optional in proto but required in schema"
                );
                assert!(
                    has_nullable_branch(prop),
                    "guest-control-proto: {schema_name}.{schema_field} lacks nullable schema branch"
                );
            } else if !field.repeated
                && scalar_type(&field.proto_type).is_some()
                && !required.contains(schema_field.as_str())
            {
                panic!(
                    "guest-control-proto: {schema_name}.{schema_field} is non-optional scalar in proto but optional in schema"
                );
            }
        }
    }
}

#[test]
fn guest_control_proto_schema_terminal_status_and_enums_match() {
    let (_proto, shape) = proto_shape();
    let schema = schema();
    let defs = definitions(&schema);

    for (enum_name, values) in &shape.enum_values {
        let node = defs.get(enum_name).unwrap_or_else(|| {
            panic!("guest-control-proto: schema missing definition for proto enum {enum_name}")
        });
        let proto_values = enum_schema_values(values, enum_name);
        let schema_values = node
            .get("enum")
            .and_then(Value::as_array)
            .expect("guest-control-proto: enum schema values missing")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            proto_values, schema_values,
            "guest-control-proto: {enum_name} enum drift"
        );
    }

    let terminal = defs
        .get("TerminalStatus")
        .expect("guest-control-proto: TerminalStatus schema missing");
    let variants = terminal
        .get("oneOf")
        .and_then(Value::as_array)
        .expect("guest-control-proto: TerminalStatus oneOf missing");
    assert_eq!(
        variants.len(),
        4,
        "guest-control-proto: TerminalStatus schema does not expose exactly four oneOf variants"
    );

    let expected_terminal = BTreeMap::from([
        (
            "exit-code",
            Field {
                name: "exit_code".to_owned(),
                proto_type: "int32".to_owned(),
                optional: false,
                repeated: false,
                number: 1,
            },
        ),
        (
            "signal",
            Field {
                name: "signal".to_owned(),
                proto_type: "uint32".to_owned(),
                optional: false,
                repeated: false,
                number: 2,
            },
        ),
        (
            "status-code",
            Field {
                name: "status_code".to_owned(),
                proto_type: "int32".to_owned(),
                optional: false,
                repeated: false,
                number: 3,
            },
        ),
        (
            "error",
            Field {
                name: "error".to_owned(),
                proto_type: "GuestControlErrorKind".to_owned(),
                optional: false,
                repeated: false,
                number: 4,
            },
        ),
    ]);

    for variant in variants {
        let props = variant
            .get("properties")
            .and_then(Value::as_object)
            .expect("guest-control-proto: TerminalStatus variant properties missing");
        let outcome = props
            .get("outcome")
            .and_then(|outcome| outcome.get("enum"))
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_str)
            .expect("guest-control-proto: TerminalStatus variant outcome discriminator missing");
        let payload = expected_terminal.get(outcome).unwrap_or_else(|| {
            panic!(
                "guest-control-proto: TerminalStatus variant has unexpected outcome discriminator"
            )
        });
        let payload_field = camel(&payload.name);
        let actual_fields = props.keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            actual_fields,
            BTreeSet::from(["outcome".to_owned(), payload_field.clone()]),
            "guest-control-proto: TerminalStatus {outcome} variant payload drift"
        );
        let prop = props.get(&payload_field).unwrap_or_else(|| {
            panic!("guest-control-proto: TerminalStatus {outcome} payload missing")
        });
        assert_scalar_shape("TerminalStatus", payload, prop, defs);
    }
}
