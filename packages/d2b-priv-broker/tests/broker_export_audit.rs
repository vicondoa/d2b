#![cfg(feature = "layer1-bootstrap")]

mod common;

use serde_json::Value;

use common::{TestBroker, audit_file_metadata};

#[test]
fn export_audit_requires_admin_and_exports_op_audit_records() {
    let broker = TestBroker::spawn("broker-export-audit-");

    broker.probe_hello(broker.d2bd_uid()).assert_success();
    broker
        .probe_stub(broker.d2bd_uid(), "ApplyNftables")
        .assert_success();

    let audit_path = broker.audit_path();
    assert!(
        audit_path.is_file(),
        "expected audit log at {}",
        audit_path.display()
    );
    let expected = (
        nix::unistd::Uid::current().as_raw(),
        nix::unistd::Gid::current().as_raw(),
        0o640,
    );
    assert_eq!(
        audit_file_metadata(&audit_path).expect("stat audit file"),
        expected,
        "expected simulated ownership/mode {expected:?}"
    );

    let write_fds = broker.audit_write_fds(&audit_path);
    assert_eq!(
        write_fds.len(),
        1,
        "expected exactly one write fd for the audit log: {write_fds:?}"
    );
    assert!(
        write_fds[0].is_append_only(),
        "audit fd is not O_APPEND: {write_fds:?}"
    );

    let unauthorized = broker.probe_export_audit(broker.d2bd_uid(), "not-authorized");
    unauthorized.assert_success();
    assert!(
        unauthorized
            .stdout()
            .contains("\"kind\":\"authz-audit-requires-admin\""),
        "expected Authz::AuditRequiresAdmin for non-admin caller role: {}",
        unauthorized.stdout()
    );

    let export = broker.probe_export_audit(broker.d2bd_uid(), "admin:9000");
    export.assert_success();
    let export_json = export.stdout();
    assert!(
        export_json.contains("ApplyNftables"),
        "admin export did not contain the denied ApplyNftables record: {export_json}"
    );
    assert!(
        !export_json.contains(&broker.scratch_path().display().to_string()),
        "exported audit data leaked a filesystem path: {export_json}"
    );

    let response: Value = serde_json::from_str(&export_json).expect("parse export response JSON");
    assert_eq!(response["response"], "ExportBrokerAuditOk");
    let lines = response["lines"]
        .as_array()
        .expect("export response lines array");
    let apply_record = lines
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["operation"] == "ApplyNftables")
        .expect("exported ApplyNftables OpAuditRecord");
    assert_eq!(apply_record["peer_uid"], broker.d2bd_uid());
    assert_eq!(apply_record["operation"], "ApplyNftables");
    assert_eq!(apply_record["public_operation_id"], "operation");
    assert_eq!(apply_record["scope_id"], "operation");
    assert_eq!(apply_record["verb"], "ApplyNftables");
    assert_eq!(apply_record["decision"], "errored");
    assert_eq!(apply_record["result"], "error");
    assert_eq!(apply_record["error_kind"], "w3-pending-typed-wire");
    assert_eq!(apply_record["operation_fields"]["target_wave"], "W3");
}
