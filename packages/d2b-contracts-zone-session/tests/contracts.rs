use d2b_contracts_zone_session::v3::{ZoneId, ZoneSpec};

#[test]
fn zone_session_contracts_preserve_zone_identity() {
    let zone = ZoneId::parse("work").expect("valid zone");
    let _ = std::any::type_name::<ZoneSpec>();
    assert_eq!(zone.as_str(), "work");
}
