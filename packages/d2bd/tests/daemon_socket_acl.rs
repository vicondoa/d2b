mod common;

mod daemon_socket_acl {
    use super::common::{
        DaemonFixture, HELLO_FRAME, TestPeer, assert_contains, spawn_d2bd_serve, test_client,
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
    fn non_launcher_uid_is_rejected() {
        run_case(
            TestPeer::deny(60001, "random-user", "users"),
            &[HELLO_FRAME],
            31,
            r#""kind":"authz-not-a-launcher""#,
            r#""type":"helloRejected""#,
        );
    }

    #[test]
    fn wheel_non_launcher_is_rejected() {
        run_case(
            TestPeer::deny(60002, "wheel-user", "wheel"),
            &[HELLO_FRAME],
            31,
            r#""kind":"authz-not-a-launcher""#,
            r#""type":"helloRejected""#,
        );
    }

    #[test]
    fn configured_launcher_is_accepted() {
        run_case(
            TestPeer::launcher(),
            &[HELLO_FRAME, AUTH_STATUS_FRAME],
            0,
            r#""type":"helloOk""#,
            r#""role":"launcher""#,
        );
    }

    #[test]
    fn daemon_self_client_is_rejected() {
        run_case(
            TestPeer::deny(0, "daemon-user", "root"),
            &[HELLO_FRAME],
            31,
            r#""kind":"authz-not-a-launcher""#,
            r#""type":"helloRejected""#,
        );
    }
}
