use std::{env, fs, path::PathBuf};

#[test]
fn production_binary_contains_no_peer_override_surface() {
    let binary = fs::read(env!("CARGO_BIN_EXE_d2bd")).expect("read production d2bd binary");
    let rendered = String::from_utf8_lossy(&binary);
    assert!(
        !rendered.contains("D2BD_TEST_PEER_"),
        "production d2bd must not contain the peer override environment surface"
    );
    assert!(
        !rendered.contains("peer_override_from_env"),
        "production d2bd must not contain the peer override implementation"
    );
    for retired in [
        "run_exec_owner",
        "dispatch_exec_management",
        "dispatch_read_guest_config",
        "exec_owner_io",
        "load_gateway_file_config",
        "relay_auth_snippet_from_config",
        "GatewayOrchestrator",
        "run_gateway_display_owner",
        "gateway_deps_from_config",
        "display_listener_from_config",
    ] {
        assert!(
            !rendered.contains(retired),
            "production d2bd must not retain retired component-session path {retired}"
        );
    }
    let source = [
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_else(|| ".".into()))
            .join("src/composition.rs"),
        env::var_os("D2B_REPO_ROOT")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("packages/d2bd/src/composition.rs"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .and_then(|path| fs::read_to_string(path).ok())
    .expect("read d2bd source");
    assert!(
        !source.contains("BrokerRequest::OpenHidrawSecurityKey"),
        "production d2bd must not own the security-key hidraw opener"
    );
}
