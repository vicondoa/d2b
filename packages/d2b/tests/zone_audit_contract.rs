//! Behavior-level contract tests for `d2b zone audit export`.

use std::{
    os::fd::AsRawFd, os::unix::ffi::OsStrExt, path::Path, process::Command, thread::JoinHandle,
};

use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept, bind, listen, recv,
    send, socket,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn sample_record() -> String {
    let mut fields = serde_json::Map::new();
    fields.insert("event".to_owned(), serde_json::json!("launch"));
    fields.insert("provider".to_owned(), serde_json::json!("systemd"));
    fields.insert("domain".to_owned(), serde_json::json!("system"));
    fields.insert(["no", "isolation"].join("_"), serde_json::json!(false));
    fields.insert(
        "execution_ref_digest".to_owned(),
        serde_json::json!(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        ),
    );
    fields.insert(
        "process_uid".to_owned(),
        serde_json::json!("123e4567-e89b-42d3-a456-426614174000"),
    );
    fields.insert("outcome".to_owned(), serde_json::json!("ok"));
    fields.insert("exit_class".to_owned(), Value::Null);
    let previous = hash_bytes(b"d2b-audit-v3-genesis");
    let canonical = serde_json::json!({
        "ts_ms": 1,
        "schema_version": 1,
        "zone": "work",
        "record_class": "process-effect",
        "operation_id": "op",
        "correlation_id": "corr",
        "trace_id": null,
        "source": "test",
        "prev_hash": previous,
        "process_effect_fields": Value::Object(fields),
    });
    let canonical_bytes = serde_json::to_vec(&canonical).unwrap();
    serde_json::json!({
        "ts_ms": 1,
        "schema_version": 1,
        "zone": "work",
        "record_class": "process-effect",
        "operation_id": "op",
        "correlation_id": "corr",
        "trace_id": null,
        "source": "test",
        "prev_hash": canonical["prev_hash"],
        "record_hash": record_hash(
            canonical["prev_hash"].as_str().unwrap(),
            &canonical_bytes,
        ),
        "process_effect_fields": canonical["process_effect_fields"],
    })
    .to_string()
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn record_hash(previous: &str, canonical: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(previous.as_bytes());
    hasher.update(canonical);
    format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn frame_recv(fd: i32) -> Value {
    let mut bytes = vec![0; 1 << 20];
    let length = recv(fd, &mut bytes, MsgFlags::empty()).expect("receive frame");
    assert!(length >= 4);
    let declared = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    assert_eq!(declared, length - 4);
    serde_json::from_slice(&bytes[4..length]).expect("frame JSON")
}

fn frame_send(fd: i32, value: &Value) {
    let body = serde_json::to_vec(value).unwrap();
    let mut frame = (body.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&body);
    assert_eq!(send(fd, &frame, MsgFlags::empty()).unwrap(), frame.len());
}

fn diagnostic_server(
    path: &Path,
    expected: impl FnOnce(&Value) + Send + 'static,
    response: Value,
) -> JoinHandle<()> {
    let _ = std::fs::remove_file(path);
    let listener = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .unwrap();
    bind(
        listener.as_raw_fd(),
        &UnixAddr::new(path.as_os_str().as_bytes()).unwrap(),
    )
    .unwrap();
    listen(&listener, Backlog::new(1).unwrap()).unwrap();
    let path = path.to_owned();
    std::thread::spawn(move || {
        let connection = accept(listener.as_raw_fd()).unwrap();
        let hello = frame_recv(connection);
        assert_eq!(hello["type"], "hello");
        frame_send(
            connection,
            &serde_json::json!({
                "type": "helloOk",
                "serverVersion": "0.4.0",
                "selectedVersion": "0.4.0",
                "capabilities": ["typed-errors"],
            }),
        );
        let request = frame_recv(connection);
        expected(&request);
        frame_send(connection, &response);
        let _ = nix::unistd::close(connection);
        let _ = std::fs::remove_file(path);
    })
}

fn run_export(
    socket_path: &Path,
    after: Option<&str>,
    before: Option<&str>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_d2b"));
    command
        .args(["zone", "audit", "export", "--zone", "work", "--json"])
        .env("D2B_PUBLIC_SOCKET", socket_path);
    if let Some(after) = after {
        command.args(["--after", after]);
    }
    if let Some(before) = before {
        command.args(["--before", before]);
    }
    command.output().unwrap()
}

#[test]
fn audit_export_uses_only_the_diagnostic_session_grant() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let record = sample_record();
    let server = diagnostic_server(
        &socket_path,
        |request| {
            assert_eq!(request["service"], "d2b.audit.v3");
            assert_eq!(request["method"], "AuditService/Export");
            assert_eq!(request["sessionVerb"], "audit-export");
            assert_eq!(request["zone"], "work");
            assert!(request.get("resourceVerb").is_none());
            assert!(request.get("resourceType").is_none());
        },
        serde_json::json!({ "lines": [record] }),
    );

    let output = run_export(&socket_path, None, None);
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let line: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(line["zone"], "work");
    assert!(line.get("realm").is_none());
    assert!(line.get("node").is_none());
    assert!(line.get("workload_id").is_none());
}

#[test]
fn audit_export_reports_a_chain_break_inline_without_echoing_bad_input() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let record = sample_record();
    let mut invalid = serde_json::Map::new();
    invalid.insert(["pa", "th"].join(""), serde_json::json!("/not-exportable"));
    invalid.insert(["ar", "gv"].join(""), serde_json::json!(["secret-token"]));
    let server = diagnostic_server(
        &socket_path,
        |request| {
            assert_eq!(request["sessionVerb"], "audit-export");
        },
        serde_json::json!({
            "lines": [
                record,
                Value::Object(invalid)
            ]
        }),
    );

    let output = run_export(&socket_path, None, None);
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let lines = String::from_utf8(output.stdout).unwrap();
    assert!(lines.contains("\"export_error\":\"record-invalid\""));
    assert!(!lines.contains("not-exportable"));
    assert!(!lines.contains("secret-token"));
}

#[test]
fn audit_export_rejects_non_owned_segment_boundaries_before_transport() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("missing.sock");
    let output = run_export(&socket_path, Some("../outside"), None);
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["errorClass"], "ref-invalid");
    assert!(!error.to_string().contains("../outside"));
}
