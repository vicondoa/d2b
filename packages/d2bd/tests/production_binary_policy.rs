use std::fs;

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
}
