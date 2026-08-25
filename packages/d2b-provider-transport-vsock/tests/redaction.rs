use d2b_contracts_resource::v3::{
    ResourceRef,
    ZoneId,
};
use d2b_provider_transport_vsock::{
    SessionKey, GuestIdentity, OpaqueBindingId, OpaqueEndpointId, PeerCid, SessionAuthority,
    SessionProof, VsockTransportSettings,
};
use ring::rand::{SystemRandom, generate};

fn nonce() -> [u8; 32] {
    generate::<[u8; 32]>(&SystemRandom::new()).unwrap().expose()
}

#[test]
fn diagnostics_do_not_expose_endpoint_binding_cid_or_signature_material() {
    let endpoint = OpaqueEndpointId::parse("endpoint-secret").unwrap();
    let binding = OpaqueBindingId::parse("binding-secret").unwrap();
    let identity = GuestIdentity::new(
        ResourceRef::parse("Guest/secret-guest").unwrap(),
        ZoneId::parse("secret-zone").unwrap(),
        PeerCid::from_core(77).unwrap(),
        "secret-boot",
    )
    .unwrap();
    let key = SessionKey::from_core([9; 32]);
    let authority = SessionAuthority::new(identity.clone(), key.clone(), 1);
    let proof = SessionProof::sign(&key, &identity, nonce(), 1);
    let rendered = format!("{endpoint:?} {binding:?} {identity:?} {proof:?} {authority:?}");
    for canary in [
        "endpoint-secret",
        "binding-secret",
        "secret-guest",
        "secret-zone",
        "77",
    ] {
        assert!(!rendered.contains(canary), "diagnostics leaked {canary}");
    }
}

#[test]
fn transport_settings_reject_raw_endpoint_fields() {
    let settings: Result<VsockTransportSettings, _> =
        serde_json::from_str(r#"{"guestRef":"Guest/guest-a","cid":42}"#);
    assert!(settings.is_err());
    let settings: Result<VsockTransportSettings, _> =
        serde_json::from_str(r#"{"guestRef":"Guest/guest-a","port":14420}"#);
    assert!(settings.is_err());
}
