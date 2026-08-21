use d2b_contracts_resource::v3::{CanonicalJsonValue, ResourceTypeName};

#[test]
fn resource_contract_surface_is_strict_and_stable() {
    let value = CanonicalJsonValue::parse(br#"{"name":"stable"}"#).expect("canonical JSON");
    assert_eq!(value.to_canonical_bytes(), br#"{"name":"stable"}"#);
    assert_eq!(ResourceTypeName::parse("Host").unwrap().as_str(), "Host");
    assert!(ResourceTypeName::parse("host").is_err());
}
