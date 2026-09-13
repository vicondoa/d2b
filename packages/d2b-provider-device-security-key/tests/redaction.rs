use d2b_provider_device_security_key::{SecurityKeyLeaseError, SecurityKeySessionId};

#[test]
fn security_key_canary_stays_out_of_debug_and_errors() {
    let session = SecurityKeySessionId::from_core([0xa5; 16]);
    let rendered = format!("{session:?}");

    assert!(!rendered.contains("a5"));
    assert!(
        SecurityKeyLeaseError::AuthorizationDenied
            .to_string()
            .starts_with("device-")
    );
}
