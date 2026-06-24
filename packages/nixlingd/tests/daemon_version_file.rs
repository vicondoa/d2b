mod common;

mod daemon_version_file {
    use std::fs;
    use std::time::Duration;

    use serde_json::Value;

    use super::common::{
        DaemonFixture, HELLO_FRAME, TestPeer, assert_contains, spawn_nixlingd_serve, test_client,
        wait_for_file,
    };

    #[test]
    fn startup_writes_version_file_next_to_public_socket() {
        let fixture = DaemonFixture::new("daemon-version-file.");
        fixture.write_config(&["launcher-user"], &["admin-user"]);
        let version_path = fixture.run_dir.join("version");

        let server = spawn_nixlingd_serve(&fixture, &TestPeer::launcher(), true, None);
        wait_for_file(&version_path, Duration::from_secs(15));

        let version: Value = serde_json::from_slice(
            &fs::read(&version_path)
                .unwrap_or_else(|err| panic!("read {}: {err}", version_path.display())),
        )
        .unwrap_or_else(|err| panic!("parse {}: {err}", version_path.display()));
        assert_eq!(
            version["serverVersion"].as_str(),
            Some(nixlingd::DEFAULT_SERVER_VERSION)
        );
        assert!(
            version["binaryPath"]
                .as_str()
                .is_some_and(|value| value.contains("nixlingd")),
            "binaryPath should identify the running nixlingd binary: {version:#}"
        );
        assert!(
            version["startedAt"]
                .as_str()
                .is_some_and(|value| value.ends_with('Z') && !value.is_empty()),
            "startedAt should carry a UTC timestamp: {version:#}"
        );
        assert_eq!(
            version["protocolVersion"],
            Value::from(u64::from(nixling_contracts::PROTOCOL_VERSION))
        );

        let (rc, output) = test_client(&fixture.socket_path, &[HELLO_FRAME]);
        let status = server.wait();
        assert!(status.success(), "nixlingd serve exited with {status:?}");
        assert_eq!(rc, 0, "hello client exit code; output:\n{output}");
        assert_contains(&output, r#""type":"helloOk""#, "hello response");
    }
}
