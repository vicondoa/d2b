mod common;

mod daemon_version_file {
    use std::fs;
    use std::time::Duration;

    use serde_json::Value;

    use super::common::{
        DaemonFixture, complete_component_session_handshake, current_username, spawn_d2bd_serve,
        wait_for_file,
    };

    #[test]
    fn startup_writes_version_file_next_to_public_socket() {
        let fixture = DaemonFixture::new("daemon-version-file.");
        let username = current_username();
        fixture.write_config(&[&username], &[&username]);
        let version_path = fixture.run_dir.join("version");

        let server = spawn_d2bd_serve(&fixture, true, None);
        wait_for_file(&version_path, Duration::from_secs(15));

        let version: Value = serde_json::from_slice(
            &fs::read(&version_path)
                .unwrap_or_else(|err| panic!("read {}: {err}", version_path.display())),
        )
        .unwrap_or_else(|err| panic!("parse {}: {err}", version_path.display()));
        assert_eq!(
            version["serverVersion"].as_str(),
            Some(d2bd::DEFAULT_SERVER_VERSION)
        );
        assert!(
            version["binaryPath"]
                .as_str()
                .is_some_and(|value| value.contains("d2bd")),
            "binaryPath should identify the running d2bd binary: {version:#}"
        );
        assert!(
            version["startedAt"]
                .as_str()
                .is_some_and(|value| value.ends_with('Z') && !value.is_empty()),
            "startedAt should carry a UTC timestamp: {version:#}"
        );
        assert_eq!(
            version["protocolVersion"],
            Value::from(u64::from(d2b_contracts::PROTOCOL_VERSION))
        );

        complete_component_session_handshake(&fixture.socket_path);
        let status = server.wait();
        assert!(status.success(), "d2bd serve exited with {status:?}");
    }
}
