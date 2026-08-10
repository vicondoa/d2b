//! Behavior-level contract tests for `d2b zone support-bundle`.

use std::{os::fd::AsRawFd, os::unix::ffi::OsStrExt, path::Path, process::Command};

use nix::sys::socket::{
    AddressFamily, Backlog, MsgFlags, SockFlag, SockType, UnixAddr, accept, bind, listen, recv,
    send, socket,
};
use serde_json::Value;

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

fn support_server(path: &Path, response: Value) -> std::thread::JoinHandle<()> {
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
        assert_eq!(request["service"], "d2b.support.v3");
        assert_eq!(request["method"], "SupportService/GenerateBundle");
        assert_eq!(request["sessionVerb"], "support-bundle");
        assert_eq!(request["zone"], "work");
        assert!(request.get("resourceVerb").is_none());
        frame_send(connection, &response);
        let _ = nix::unistd::close(connection);
        let _ = std::fs::remove_file(path);
    })
}

fn run_support_bundle(socket_path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["zone", "support-bundle", "--zone", "work"])
        .env("D2B_PUBLIC_SOCKET", socket_path)
        .output()
        .unwrap()
}

#[test]
fn support_bundle_is_admin_only_and_redacts_bounded_status_fields() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let server = support_server(
        &socket_path,
        serde_json::json!({
            "bundle": {
                "bundle_completeness": "partial",
                "quarantined": true,
                "doctor": {
                    "zone_phase": "Failed",
                    "store_health": {
                        "phase": "quarantined",
                        "revision": 1,
                        "compaction_floor": 1,
                        "watch_active": 0
                    },
                    "audit": {"phase": "unavailable"},
                    "telemetry": {"phase": "unavailable"},
                    "schema_catalog_consistent": true,
                    "watch_quota_headroom": true,
                    "audit_hash_chain_clean": false
                },
                "resource_status": [{
                    "metadata": {
                        "name": "secret-resource-name",
                        "uid": "Provider:opaque",
                        "zone": "work",
                        "generation": 2,
                        "revision": 3,
                        "observedAt": "tick-2"
                    },
                    "spec": {"credential": "must-not-escape"},
                    "status": {
                        "phase": "degraded",
                        "conditions": ["store-quarantined"],
                        "observedGeneration": 2,
                        "outcome": "degraded"
                    }
                }],
                "controllers": [{
                    "handler": "provider",
                    "phase": "degraded",
                    "queue_depth": 2
                }],
                "schema_catalog": [["Provider", "v3"]],
                "audit_segments": [{
                    "filename": "audit-20240101000000000000.jsonl",
                    "bytes": 12,
                    "records": 1
                }],
                "telemetry": {
                    "phase": "unavailable",
                    "exported": 0,
                    "dropped": 2
                },
                "logs": [{
                    "event": "collector-export",
                    "outcome": "dropped",
                    "timestamp": "tick-2",
                    "path": "/private"
                }]
            }
        }),
    );

    let output = run_support_bundle(&socket_path);
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let bundle: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(bundle["bundle_completeness"], "partial");
    assert_eq!(bundle["resource_status"].as_array().unwrap().len(), 1);
    assert!(bundle.get("spec").is_none());
    assert!(bundle.to_string().find("metadata.name").is_none());
    assert!(!bundle.to_string().contains("secret-resource-name"));
    assert!(!bundle.to_string().contains("must-not-escape"));
    assert!(!bundle.to_string().contains("/private"));
}

#[test]
fn support_bundle_rejects_an_unavailable_admin_session() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let server = support_server(
        &socket_path,
        serde_json::json!({
            "type": "error",
            "errorClass": "authorization-denied",
            "message": "support bundle requires an admin role"
        }),
    );
    let output = run_support_bundle(&socket_path);
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["errorClass"], "authorization-denied");
    assert!(!error.to_string().contains("support bundle requires"));
}
