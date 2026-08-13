mod common;

mod daemon_socket_acl {
    use super::common::{
        DaemonFixture, HELLO_FRAME, TestPeer, assert_contains, lifecycle_group_name,
        set_public_socket_group, spawn_d2bd_serve, spawn_d2bd_serve_with_forged_peer_env,
        test_client,
    };

    const AUTH_STATUS_FRAME: &str = r#"{"type":"authStatus"}"#;

    fn run_case(peer: TestPeer, frames: &[&str], expect_rc: i32, expect_a: &str, expect_b: &str) {
        let fixture = DaemonFixture::new("daemon-socket-acl.");
        fixture.write_config(&["launcher-user"], &["admin-user"]);
        let server = spawn_d2bd_serve(&fixture, &peer, true, None);

        let (rc, output) = test_client(&fixture.socket_path, frames);
        let status = server.wait();
        assert!(status.success(), "d2bd serve exited with {status:?}");
        assert_eq!(
            rc, expect_rc,
            "daemon public-socket ACL exit code; output:\n{output}"
        );
        assert_contains(&output, expect_a, "primary match");
        assert_contains(&output, expect_b, "secondary match");
    }

    #[test]
    fn real_current_peer_missing_from_classifier_is_rejected() {
        run_case(
            TestPeer::deny(60001, "random-user", "users"),
            &[HELLO_FRAME],
            31,
            r#""kind":"authz-not-a-launcher""#,
            r#""type":"helloRejected""#,
        );
    }

    #[test]
    fn real_current_peer_is_denied_when_configured_group_is_unrelated() {
        run_case(
            TestPeer::deny(60002, "wheel-user", "wheel"),
            &[HELLO_FRAME],
            31,
            r#""kind":"authz-not-a-launcher""#,
            r#""type":"helloRejected""#,
        );
    }

    #[test]
    fn real_current_peer_in_configured_classifier_is_accepted() {
        run_case(
            TestPeer::launcher(),
            &[HELLO_FRAME, AUTH_STATUS_FRAME],
            0,
            r#""type":"helloOk""#,
            r#""role":"launcher""#,
        );
    }

    #[test]
    fn forged_peer_environment_cannot_authorize_real_socket_peer() {
        let fixture = DaemonFixture::new("daemon-socket-forged-peer.");
        fixture.write_config(&["launcher-user", "admin-user"], &["admin-user"]);
        set_public_socket_group(&fixture, "d2b-test-forged-peer");
        // The daemon receives an environment value claiming that the client
        // is an administrator, but the test-client is a real local process.
        // Production admission must use that connection's SO_PEERCRED.
        let server =
            spawn_d2bd_serve_with_forged_peer_env(&fixture, &TestPeer::admin(), true, None);

        let (rc, output) = test_client(&fixture.socket_path, &[HELLO_FRAME]);
        let status = server.wait();
        assert!(status.success(), "d2bd serve exited with {status:?}");
        assert_eq!(
            rc, 31,
            "forged peer environment must not authorize:\n{output}"
        );
        assert_contains(
            &output,
            r#""kind":"authz-not-a-launcher""#,
            "real peer is denied",
        );
        assert_contains(&output, r#""type":"helloRejected""#, "rejection frame");
    }

    #[test]
    fn configured_lifecycle_group_grants_real_peer_access() {
        let fixture = DaemonFixture::new("daemon-socket-lifecycle-group.");
        fixture.write_config(&["unrelated-user"], &[]);
        set_public_socket_group(&fixture, &lifecycle_group_name());
        let server = spawn_d2bd_serve_with_forged_peer_env(
            &fixture,
            &TestPeer::deny(60001, "random-user", "wheel"),
            true,
            None,
        );

        let (rc, output) = test_client(&fixture.socket_path, &[HELLO_FRAME]);
        let status = server.wait();
        assert!(status.success(), "d2bd serve exited with {status:?}");
        assert_eq!(rc, 0, "configured lifecycle group grants access:\n{output}");
        assert_contains(&output, r#""type":"helloOk""#, "accepted frame");
    }

    #[test]
    fn real_current_peer_matching_daemon_uid_is_rejected() {
        run_case(
            TestPeer::daemon(),
            &[HELLO_FRAME],
            31,
            r#""kind":"authz-not-a-launcher""#,
            r#""type":"helloRejected""#,
        );
    }
}
