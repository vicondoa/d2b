//! Behavior-level contract tests for `d2b zone doctor`.

use std::{
    os::fd::AsRawFd, os::unix::ffi::OsStrExt, path::Path, process::Command, thread::JoinHandle,
};

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

fn status_server(path: &Path, response: Value) -> JoinHandle<()> {
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
        assert_eq!(request["service"], "d2b.zone.v3");
        assert_eq!(request["method"], "ZoneStatus");
        assert!(request.get("sessionVerb").is_none());
        assert_eq!(request["doctor"], true);
        frame_send(connection, &response);
        let _ = nix::unistd::close(connection);
        let _ = std::fs::remove_file(path);
    })
}

fn healthy_status(telemetry: Value) -> Value {
    serde_json::json!({
        "zone_phase": "Ready",
        "store_health": {
            "phase": "ready",
            "revision": 10,
            "compaction_floor": 1,
            "watch_active": 0
        },
        "controllers": [{
            "handler": "provider",
            "phase": "ready",
            "queue_depth": 0,
            "last_reconciled_at": "tick-1"
        }],
        "providers": [{
            "provider": "system-core",
            "phase": "ready",
            "component_phases": {}
        }],
        "process_counts": {"active": 1, "failed": 0},
        "audit": {
            "phase": "ok",
            "segments": 1,
            "drop_privileged": 0,
            "drop_total": 0,
            "defects": []
        },
        "telemetry": telemetry,
        "schema_catalog_consistent": true,
        "watch_quota_headroom": true,
        "audit_hash_chain_clean": true
    })
}

fn run_doctor(socket_path: &Path, manifest: Option<&Path>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_d2b"));
    command
        .args(["zone", "doctor", "--zone", "work", "--json"])
        .env("D2B_PUBLIC_SOCKET", socket_path);
    if let Some(manifest) = manifest {
        command.env("D2B_MANIFEST_PATH", manifest);
    }
    command.output().unwrap()
}

fn run_doctor_with_audit_dir(socket_path: &Path, audit_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d2b"))
        .args(["zone", "doctor", "--zone", "work", "--json"])
        .env("D2B_PUBLIC_SOCKET", socket_path)
        .env("D2B_ZONE_AUDIT_DIR", audit_dir)
        .output()
        .unwrap()
}

#[test]
fn doctor_returns_zero_for_a_ready_zone_and_uses_a_resource_read() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let server = status_server(
        &socket_path,
        healthy_status(serde_json::json!({
            "phase": "ok",
            "drop_total": 0
        })),
    );
    let output = run_doctor(&socket_path, None);
    server.join().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["zone"], "work");
    assert_eq!(report["zone_phase"], "Ready");
    assert_eq!(report["summary"]["error"], 0);
    assert!(report.get("broker_ready").is_none());
}

#[test]
fn doctor_treats_absent_otel_as_a_warning_without_host_probe_side_effects() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let manifest = temporary.path().join("manifest.json");
    std::fs::write(&manifest, r#"{"_observability":{"enabled":false}}"#).unwrap();
    let server = status_server(
        &socket_path,
        healthy_status(serde_json::json!({"phase": "unavailable", "drop_total": 1})),
    );
    let output = run_doctor(&socket_path, Some(&manifest));
    server.join().unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["telemetry"]["phase"], "unavailable");
    assert_eq!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "otel-sink-reachable")
            .unwrap()["status"],
        "warn"
    );
    assert!(
        !output
            .stdout
            .windows(b"broker_ready".len())
            .any(|window| { window == b"broker_ready" })
    );
}

#[test]
fn doctor_reads_a_redirected_audit_inventory_and_reports_a_chain_break() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let audit_dir = temporary.path().join("audit");
    std::fs::create_dir(&audit_dir).unwrap();
    std::fs::write(
        audit_dir.join("audit-20240101000000000000.jsonl"),
        b"not an audit record\n",
    )
    .unwrap();
    let server = status_server(
        &socket_path,
        healthy_status(serde_json::json!({
            "phase": "ok",
            "drop_total": 0
        })),
    );
    let output = run_doctor_with_audit_dir(&socket_path, &audit_dir);
    server.join().unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["audit"]["segments"], 1);
    assert_eq!(report["audit"]["phase"], "ok");
    assert_eq!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == "audit-hash-chain-clean")
            .unwrap()["status"],
        "error"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("audit-202401"));
}

#[test]
fn doctor_returns_one_for_a_quarantined_zone_and_keeps_details_redacted() {
    let temporary = tempfile::tempdir().unwrap();
    let socket_path = temporary.path().join("public.sock");
    let server = status_server(
        &socket_path,
        serde_json::json!({
            "zone_phase": "Failed",
            "store_health": {
                "phase": "quarantined",
                "revision": 4,
                "compaction_floor": 4,
                "watch_active": 0
            },
            "audit": {"phase": "unavailable", "segments": 1},
            "telemetry": {"phase": "unavailable", "drop_total": 4},
            "schema_catalog_consistent": true,
            "watch_quota_headroom": true,
            "audit_hash_chain_clean": false
        }),
    );
    let output = run_doctor(&socket_path, None);
    server.join().unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["zone_phase"], "Failed");
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "audit-hash-chain-clean")
    );
    assert!(
        !output
            .stdout
            .windows(b"/var/".len())
            .any(|window| { window == b"/var/" })
    );
}
