use d2b_contracts_resource::v3::{
    BoundedText, BridgeAttributionQuality, BridgeEndpointIdentity, BridgeFrame, CanonicalJsonValue,
    ResourceGeneration, ResourceRef, ResourceTypeName, ResourceUid, WorkloadProviderKind, ZoneId,
    ZoneResourceIdentity, ZoneRevision,
};

#[test]
fn resource_contract_surface_is_strict_and_stable() {
    let value = CanonicalJsonValue::parse(br#"{"name":"stable"}"#).expect("canonical JSON");
    assert_eq!(value.to_canonical_bytes(), br#"{"name":"stable"}"#);
    assert_eq!(ResourceTypeName::parse("Host").unwrap().as_str(), "Host");
    assert!(ResourceTypeName::parse("host").is_err());
}

const ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const OTHER_ZONE_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const GUEST_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const OTHER_GUEST_UID: &str = "423e4567-e89b-42d3-a456-426614174003";

fn identity(
    zone: &str,
    zone_uid: &str,
    resource_uid: &str,
    generation: u64,
    revision: u64,
) -> ZoneResourceIdentity {
    ZoneResourceIdentity::new(
        ZoneId::parse(zone).unwrap(),
        ResourceUid::parse(zone_uid).unwrap(),
        ResourceRef::parse("Guest/browser").unwrap(),
        ResourceUid::parse(resource_uid).unwrap(),
        ResourceGeneration::new(generation).unwrap(),
        ZoneRevision::new(revision),
    )
}

#[test]
fn zone_resource_identity_round_trips_and_binds_immutable_uids() {
    let resource_identity = identity("work", ZONE_UID, GUEST_UID, 3, 9);
    let same_name_other_zone = identity("personal", OTHER_ZONE_UID, OTHER_GUEST_UID, 3, 9);

    assert_ne!(resource_identity, same_name_other_zone);
    assert!(resource_identity.matches(
        &ZoneId::parse("work").unwrap(),
        &ResourceUid::parse(ZONE_UID).unwrap(),
        &ResourceRef::parse("Guest/browser").unwrap(),
        &ResourceUid::parse(GUEST_UID).unwrap(),
        ResourceGeneration::new(3).unwrap(),
        ZoneRevision::new(9),
    ));

    let json = serde_json::to_value(&resource_identity).expect("serialize identity");
    assert_eq!(json["zone"], "work");
    assert_eq!(json["zoneUid"], ZONE_UID);
    assert_eq!(json["resourceRef"], "Guest/browser");
    assert_eq!(json["resourceUid"], GUEST_UID);
    assert_eq!(json["generation"], 3);
    assert_eq!(json["revision"], 9);
    assert!(json.get("realmId").is_none());
    assert!(json.get("realmPath").is_none());
    assert!(json.get("canonicalTarget").is_none());
    assert_eq!(
        format!("{resource_identity:?}"),
        "ZoneResourceIdentity(<redacted>)"
    );

    let decoded: ZoneResourceIdentity = serde_json::from_value(json).expect("deserialize identity");
    assert_eq!(decoded, resource_identity);
}

#[test]
fn zone_resource_identity_rejects_stale_uid_generation_and_legacy_fields() {
    let identity = identity("work", ZONE_UID, GUEST_UID, 3, 9);
    let zone = ZoneId::parse("work").unwrap();
    let zone_uid = ResourceUid::parse(ZONE_UID).unwrap();
    let resource_ref = ResourceRef::parse("Guest/browser").unwrap();
    let resource_uid = ResourceUid::parse(GUEST_UID).unwrap();
    let generation = ResourceGeneration::new(3).unwrap();
    let revision = ZoneRevision::new(9);

    assert!(!identity.matches(
        &zone,
        &zone_uid,
        &resource_ref,
        &ResourceUid::parse(OTHER_GUEST_UID).unwrap(),
        generation,
        revision,
    ));
    assert!(!identity.matches(
        &zone,
        &zone_uid,
        &resource_ref,
        &resource_uid,
        ResourceGeneration::new(4).unwrap(),
        revision,
    ));
    assert!(!identity.matches(
        &zone,
        &ResourceUid::parse(OTHER_ZONE_UID).unwrap(),
        &resource_ref,
        &resource_uid,
        generation,
        revision,
    ));
    assert!(!identity.matches(
        &zone,
        &zone_uid,
        &resource_ref,
        &resource_uid,
        generation,
        ZoneRevision::new(10),
    ));

    let mut legacy = serde_json::to_value(&identity).expect("serialize identity");
    legacy["realmId"] = serde_json::json!("work");
    legacy["realmPath"] = serde_json::json!(["work"]);
    legacy["canonicalTarget"] = serde_json::json!("browser.work.d2b");
    assert!(
        serde_json::from_value::<ZoneResourceIdentity>(legacy).is_err(),
        "retired realm identity cannot be expressed by the Zone contract"
    );
}

#[test]
fn bridge_frame_is_zone_native_and_serializes_attribution_once() {
    let endpoint = BridgeEndpointIdentity::new(
        identity("work", ZONE_UID, GUEST_UID, 3, 9),
        WorkloadProviderKind::LocalVm,
    );
    let frame = BridgeFrame::PasteRequest {
        endpoint,
        mime_type: BoundedText::parse("text/plain").unwrap(),
        source_id: 7,
        source_attribution: BridgeAttributionQuality::ExactClient,
    };

    let json = serde_json::to_value(&frame).expect("serialize bridge frame");
    assert_eq!(json["type"], "paste_request");
    assert_eq!(json["endpoint"]["resource"]["resourceRef"], "Guest/browser");
    assert_eq!(json["endpoint"]["providerKind"], "local-vm");
    assert!(json["endpoint"].get("canonicalTarget").is_none());
    assert!(json["endpoint"].get("legacyVmName").is_none());

    let mut unknown = json.clone();
    unknown["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BridgeFrame>(unknown).is_err());

    let decoded: BridgeFrame = serde_json::from_value(json).expect("deserialize bridge frame");
    assert_eq!(decoded, frame);
}
