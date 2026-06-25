use nixling_contract_tests::read_repo_file;
use nixling_contracts::{
    guest_wire::{
        GUEST_CONTROL_PROTOCOL_VERSION, GuestRequestMetadata, GuestUsbipBusId, GuestUsbipHost,
        GuestVmId, RequestId, UsbipStatusEntry, UsbipStatusRequest, UsbipStatusResponse,
    },
    public_wire::{
        PublicResponse, UsbProbeEntryKind, UsbipProbeEntry, UsbipProbeResponse, UsbipProbeStatus,
    },
    types::MediaRef,
};
use serde_json::{Value, json};

const GUEST_CONTROL_SCHEMA_REL: &str = "docs/reference/schemas/v2/guest-control.json";
const WIRE_PROTOCOL_SCHEMA_REL: &str = "docs/reference/schemas/v2/wire-protocol.json";

fn schema(rel: &str) -> Value {
    serde_json::from_str(&read_repo_file(rel))
        .unwrap_or_else(|err| panic!("{rel} must parse as JSON schema: {err}"))
}

fn resolve<'a>(mut node: &'a Value, root: &'a Value) -> &'a Value {
    while let Some(reference) = node.get("$ref").and_then(Value::as_str) {
        let ptr = reference
            .strip_prefix("#/")
            .unwrap_or_else(|| panic!("external $ref not supported in contract test: {reference}"));
        let mut cur = root;
        for part in ptr.split('/') {
            let key = part.replace("~1", "/").replace("~0", "~");
            cur = cur
                .get(&key)
                .unwrap_or_else(|| panic!("dangling schema $ref {reference} at {key}"));
        }
        node = cur;
    }
    node
}

fn definition<'a>(root: &'a Value, name: &str) -> &'a Value {
    root.get("definitions")
        .and_then(Value::as_object)
        .and_then(|defs| defs.get(name))
        .unwrap_or_else(|| panic!("schema missing definition {name}"))
}

fn properties(node: &Value) -> &serde_json::Map<String, Value> {
    node.get("properties")
        .and_then(Value::as_object)
        .expect("object schema properties")
}

fn required(node: &Value) -> Vec<&str> {
    node.get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn has_null_branch(node: &Value) -> bool {
    match node.get("type") {
        Some(Value::String(kind)) if kind == "null" => return true,
        Some(Value::Array(kinds)) if kinds.iter().any(|kind| kind.as_str() == Some("null")) => {
            return true;
        }
        _ => {}
    }
    node.get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|variants| variants.iter().any(has_null_branch))
}

fn assert_string_def(root: &Value, name: &str, max_length: u64) {
    let node = resolve(definition(root, name), root);
    assert_eq!(node.get("type").and_then(Value::as_str), Some("string"));
    assert_eq!(node.get("minLength").and_then(Value::as_u64), Some(1));
    assert_eq!(
        node.get("maxLength").and_then(Value::as_u64),
        Some(max_length)
    );
}

#[test]
fn guest_control_usb_status_json_schema_matches_dto_semantics() {
    let metadata = GuestRequestMetadata {
        vm_id: GuestVmId::new("corp-vm"),
        request_id: RequestId::new("req-usbip-status"),
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
    };
    let request = UsbipStatusRequest {
        metadata,
        host: None,
        bus_id: Some(GuestUsbipBusId::new("1-2.1")),
    };
    assert_eq!(
        serde_json::to_value(&request).expect("request serializes"),
        json!({
            "metadata": {
                "vmId": "corp-vm",
                "requestId": "req-usbip-status",
                "protocolVersion": GUEST_CONTROL_PROTOCOL_VERSION
            },
            "host": null,
            "busId": "1-2.1"
        })
    );
    let rejected_unknown = serde_json::from_value::<UsbipStatusRequest>(json!({
        "metadata": {
            "vmId": "corp-vm",
            "requestId": "req-usbip-status",
            "protocolVersion": GUEST_CONTROL_PROTOCOL_VERSION
        },
        "unexpected": true
    }));
    assert!(
        rejected_unknown.is_err(),
        "guest USB status request rejects unknown fields"
    );

    let response = UsbipStatusResponse {
        imports: vec![UsbipStatusEntry {
            port: 0,
            host: GuestUsbipHost::new("192.0.2.1"),
            tcp_port: 3240,
            bus_id: GuestUsbipBusId::new("1-2.1"),
        }],
        error: None,
    };
    assert_eq!(
        serde_json::to_value(&response).expect("response serializes"),
        json!({
            "imports": [{
                "port": 0,
                "host": "192.0.2.1",
                "tcpPort": 3240,
                "busId": "1-2.1"
            }],
            "error": null
        })
    );

    let root = schema(GUEST_CONTROL_SCHEMA_REL);
    let root_required = required(&root);
    assert!(root_required.contains(&"usbipStatus"));
    assert!(root_required.contains(&"usbipStatusResult"));
    assert_eq!(
        resolve(
            properties(&root)
                .get("usbipStatus")
                .expect("root usbipStatus property"),
            &root
        ),
        definition(&root, "UsbipStatusRequest")
    );
    assert_eq!(
        resolve(
            properties(&root)
                .get("usbipStatusResult")
                .expect("root usbipStatusResult property"),
            &root
        ),
        definition(&root, "UsbipStatusResponse")
    );

    assert_string_def(&root, "GuestUsbipHost", 64);
    assert_string_def(&root, "GuestUsbipBusId", 31);

    let request_def = resolve(definition(&root, "UsbipStatusRequest"), &root);
    assert_eq!(required(request_def), vec!["metadata"]);
    let request_props = properties(request_def);
    for field in ["host", "busId"] {
        assert!(
            has_null_branch(
                request_props
                    .get(field)
                    .expect("optional USB status filter")
            ),
            "UsbipStatusRequest.{field} must be nullable"
        );
    }
    assert_eq!(
        request_def.get("additionalProperties"),
        Some(&Value::Bool(false))
    );

    let response_def = resolve(definition(&root, "UsbipStatusResponse"), &root);
    assert_eq!(required(response_def), vec!["imports"]);
    let response_props = properties(response_def);
    assert_eq!(
        response_props
            .get("imports")
            .and_then(|node| node.get("maxItems"))
            .and_then(Value::as_u64),
        Some(64)
    );
    assert!(
        has_null_branch(response_props.get("error").expect("nullable status error")),
        "UsbipStatusResponse.error must remain nullable and optional"
    );

    let entry_def = resolve(definition(&root, "UsbipStatusEntry"), &root);
    assert_eq!(
        required(entry_def),
        vec!["busId", "host", "port", "tcpPort"]
    );
    let entry_props = properties(entry_def);
    assert_eq!(
        entry_props
            .get("port")
            .and_then(|node| node.get("minimum"))
            .and_then(Value::as_f64),
        Some(0.0)
    );
    assert_eq!(
        entry_props
            .get("port")
            .and_then(|node| node.get("maximum"))
            .and_then(Value::as_f64),
        Some(65_535.0)
    );
    assert_eq!(
        entry_props
            .get("tcpPort")
            .and_then(|node| node.get("minimum"))
            .and_then(Value::as_f64),
        Some(1.0)
    );
    assert_eq!(
        entry_props
            .get("tcpPort")
            .and_then(|node| node.get("maximum"))
            .and_then(Value::as_f64),
        Some(65_535.0)
    );
}

#[test]
fn public_usb_probe_json_schema_matches_dto_semantics() {
    let payload = UsbipProbeResponse {
        entries: vec![
            UsbipProbeEntry {
                kind: UsbProbeEntryKind::Usbip,
                vm: "corp-vm".to_owned(),
                env: "work".to_owned(),
                bus_id: "1-2".to_owned(),
                lock_path: "/run/nixling/locks/usbip/1-2".to_owned(),
                status: UsbipProbeStatus::Bound,
                owner_vm: None,
                slot: None,
                media_ref: None,
                source_kind: None,
                candidate_bus_ids: Vec::new(),
                follow_up_command: None,
                durable_claim: Default::default(),
                host: Default::default(),
                guest: Default::default(),
                topology_policy: Default::default(),
                degraded_reasons: Vec::new(),
                remediation_commands: Vec::new(),
            },
            UsbipProbeEntry {
                kind: UsbProbeEntryKind::QemuMediaSlot,
                vm: "media".to_owned(),
                env: "work".to_owned(),
                bus_id: "1-2.3".to_owned(),
                lock_path: "/run/nixling/locks/usbip/1-2.3".to_owned(),
                status: UsbipProbeStatus::Enrollable,
                owner_vm: Some("media".to_owned()),
                slot: Some("installer".to_owned()),
                media_ref: Some(MediaRef::new("installer-usb")),
                source_kind: Some("by-id-name".to_owned()),
                candidate_bus_ids: vec!["1-2.3".to_owned()],
                follow_up_command: Some("nixling usb attach media 1-2.3 --apply".to_owned()),
                durable_claim: Default::default(),
                host: Default::default(),
                guest: Default::default(),
                topology_policy: Default::default(),
                degraded_reasons: Vec::new(),
                remediation_commands: Vec::new(),
            },
        ],
    };
    let response = PublicResponse::UsbipProbe(payload.clone());

    let value = serde_json::to_value(&response).expect("public USB probe response serializes");
    assert_eq!(value["kind"], "usb probe");
    let entries = value["payload"]["entries"]
        .as_array()
        .expect("entries array");
    assert!(
        entries[0].get("kind").is_none(),
        "default usbip probe entry kind is intentionally omitted from JSON"
    );
    assert_eq!(entries[0]["status"], "bound");
    assert!(
        entries[0].get("candidateBusIds").is_none(),
        "empty candidate bus-id list is omitted from JSON"
    );
    assert_eq!(entries[1]["kind"], "qemu-media-slot");
    assert_eq!(entries[1]["status"], "enrollable");
    assert_eq!(entries[1]["mediaRef"], "installer-usb");
    assert_eq!(
        serde_json::from_value::<UsbipProbeResponse>(value["payload"].clone())
            .expect("response payload deserializes"),
        payload
    );

    let root = schema(WIRE_PROTOCOL_SCHEMA_REL);
    let response_def = resolve(definition(&root, "UsbipProbeResponse"), &root);
    assert_eq!(required(response_def), vec!["entries"]);
    let entry_def = resolve(definition(&root, "UsbipProbeEntry"), &root);
    assert_eq!(
        required(entry_def),
        vec!["busId", "env", "lockPath", "status", "vm"]
    );
    let entry_props = properties(entry_def);
    for optional in [
        "kind",
        "ownerVm",
        "slot",
        "mediaRef",
        "sourceKind",
        "candidateBusIds",
        "followUpCommand",
    ] {
        assert!(
            entry_props.contains_key(optional),
            "UsbipProbeEntry schema missing optional {optional}"
        );
        assert!(
            !required(entry_def).contains(&optional),
            "UsbipProbeEntry.{optional} must stay optional in public JSON"
        );
    }
    let kind_values = resolve(definition(&root, "UsbProbeEntryKind"), &root)
        .get("enum")
        .and_then(Value::as_array)
        .expect("UsbProbeEntryKind enum");
    assert_eq!(kind_values, &vec![json!("usbip"), json!("qemu-media-slot")]);
    let status_values = resolve(definition(&root, "UsbipProbeStatus"), &root)
        .get("enum")
        .and_then(Value::as_array)
        .expect("UsbipProbeStatus enum");
    for status in [
        "bound",
        "unbound",
        "enrollable",
        "enrolled",
        "stale",
        "direct-config",
    ] {
        assert!(
            status_values.contains(&json!(status)),
            "UsbipProbeStatus schema missing {status}"
        );
    }
}
