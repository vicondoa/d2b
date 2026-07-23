use std::fs;
use std::process::Command;

#[test]
fn check_config_validates_json_and_paths_without_runtime_side_effects() {
    let temp = std::env::temp_dir().join(format!(
        "d2b-clipd-test-{}-{}.json",
        std::process::id(),
        unique_suffix()
    ));
    fs::write(&temp, r#"{"version":1}"#).expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_d2b-clipd"))
        .arg("--config")
        .arg(&temp)
        .arg("--bridge-root")
        .arg("/run/d2b/clipd")
        .arg("--check-config")
        .output()
        .expect("spawn d2b-clipd");
    let _ = fs::remove_file(&temp);

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("config ok"));
}

#[test]
fn check_config_rejects_relative_picker_path() {
    let temp = std::env::temp_dir().join(format!(
        "d2b-clipd-test-{}-{}.json",
        std::process::id(),
        unique_suffix()
    ));
    fs::write(&temp, r#"{"version":1}"#).expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_d2b-clipd"))
        .arg("--config")
        .arg(&temp)
        .arg("--bridge-root")
        .arg("/run/d2b/clipd")
        .arg("--picker")
        .arg("relative-picker")
        .arg("--check-config")
        .output()
        .expect("spawn d2b-clipd");
    let _ = fs::remove_file(&temp);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be absolute"));
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos()
}

/// Guard against the thread::park() scaffold pattern in d2b-clipd/src/main.rs.
///
/// A real clipboard authority daemon uses an event loop (epoll / tokio / etc.).
/// If this test fails, replace the `std::thread::park()` call with real
/// daemon initialization that serves picker IPC and bridge sockets.
#[test]
fn clipd_is_not_a_thread_park_scaffold() {
    let main_src = include_str!("../src/main.rs");
    assert!(
        !main_src.contains("thread::park()"),
        "d2b-clipd/src/main.rs uses thread::park() — this is a scaffold placeholder. \
         Implement the real daemon event loop (picker IPC + bridge socket serving). \
         ADR 0042 requires d2b-clipd to own picker supervision and transfer FD dispatch."
    );
}

/// Verify the picker binary does a valid ADR 0042 protocol handshake.
///
/// Set CLIPD_TEST_PICKER to the absolute path of a compiled d2b-clip-picker
/// binary to enable this test. In CI the picker is built from the sibling
/// d2b-clip-picker repo and the env var is injected by the test harness.
/// Without the env var the test reports skip-friendly success.
#[test]
fn picker_protocol_handshake_proves_picker_works() {
    let picker_path = match std::env::var("CLIPD_TEST_PICKER") {
        Ok(p) if !p.is_empty() => p,
        _ => return, // not configured — skip
    };

    use std::io::{BufRead, BufReader};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;

    let (parent, child) = UnixStream::pair().expect("socketpair");

    // Get the raw fd number to pass to the picker binary via --ipc-fd.
    // We leak the child fd into the child process; it will be closed when
    // the child process exits.
    let child_fd_num = child.as_raw_fd();

    let mut child_proc = Command::new(&picker_path)
        .arg("--ipc-fd")
        .arg(child_fd_num.to_string())
        .spawn()
        .expect("spawn picker binary");

    // Keep `child` alive in the parent long enough for the spawn to succeed,
    // then drop it so the picker sees the parent socket as the only writer.
    drop(child);

    // The picker must send a client_hello frame immediately.
    parent
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("set read timeout");
    let mut reader = BufReader::new(&parent);
    let mut hello_line = String::new();
    match reader.read_line(&mut hello_line) {
        Ok(0) => panic!("picker closed connection without sending client_hello"),
        Ok(_) => {}
        Err(err) => panic!("failed to read picker client_hello within 5 s: {err}"),
    }
    let v: serde_json::Value = serde_json::from_str(hello_line.trim_end())
        .expect("picker client_hello must be valid JSON");
    assert_eq!(
        v["type"], "client_hello",
        "picker first frame must be client_hello, got: {hello_line}"
    );
    assert!(
        v.get("protocol_version_range").is_some(),
        "client_hello must include protocol_version_range"
    );
    assert!(
        v.get("picker_version").is_some(),
        "client_hello must include picker_version"
    );

    // Signal the picker to exit cleanly by closing the parent socket.
    drop(parent);
    let _ = child_proc.wait();
}
