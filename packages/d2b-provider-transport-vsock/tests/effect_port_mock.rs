use d2b_provider_transport_vsock::{OpaqueBindingId, OpaqueEndpointId, VsockEffectError};

#[test]
fn opaque_effect_ids_are_bounded_and_redacted() {
    assert!(OpaqueEndpointId::parse("endpoint-a").is_ok());
    assert!(OpaqueBindingId::parse("binding-a").is_ok());
    assert_eq!(
        OpaqueEndpointId::parse("Endpoint-a").unwrap_err(),
        VsockEffectError::EffectRejected
    );
    assert_eq!(
        OpaqueBindingId::parse("binding/path").unwrap_err(),
        VsockEffectError::EffectRejected
    );
    assert_eq!(
        format!("{:?}", OpaqueEndpointId::parse("endpoint-a").unwrap()),
        "OpaqueEndpointId(<redacted>)"
    );
}
