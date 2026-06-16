mod common;

mod daemon_version_negotiation {
    use super::common::{
        assert_contains, spawn_nixlingd_serve, test_client, DaemonFixture, TestPeer, HELLO_FRAME,
    };

    fn run_case(frames: &[&str], expect_rc: i32, expect_a: &str, expect_b: &str) {
        let fixture = DaemonFixture::new("daemon-version-negotiation.");
        fixture.write_config(&["launcher-user"], &["admin-user"]);
        let server = spawn_nixlingd_serve(&fixture, &TestPeer::launcher(), true, None);

        let (rc, output) = test_client(&fixture.socket_path, frames);
        let status = server.wait();
        assert!(status.success(), "nixlingd serve exited with {status:?}");
        assert_eq!(rc, expect_rc, "daemon version-negotiation exit code");
        assert_contains(&output, expect_a, "primary match");
        assert_contains(&output, expect_b, "secondary match");
    }

    #[test]
    fn version_mismatch_is_rejected() {
        run_case(
            &[r#"{"type":"hello","clientVersion":"<0.4.0","supportedFeatures":[]}"#],
            52,
            r#""reason":"versionMismatch""#,
            r#""kind":"wire-version-mismatch""#,
        );
    }

    #[test]
    fn unknown_feature_flags_are_accepted() {
        run_case(
            &[
                r#"{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":["future-flag","future-flag-2"]}"#,
                r#"{"type":"authStatus"}"#,
            ],
            0,
            r#""type":"helloOk""#,
            r#""type":"authStatusResponse""#,
        );
    }

    #[test]
    fn unknown_hello_field_is_rejected() {
        run_case(
            &[
                r#"{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[],"unexpected":true}"#,
            ],
            51,
            r#""type":"helloRejected""#,
            r#""kind":"wire-unknown-field""#,
        );
    }

    #[test]
    fn invalid_ifname_is_rejected() {
        run_case(
            &[
                HELLO_FRAME,
                r#"{"type":"hostCheck","strict":false,"ifName":"abcdefghijklmnop"}"#,
            ],
            53,
            r#""kind":"wire-ifname-invalid""#,
            r#""type":"error""#,
        );
    }
}
