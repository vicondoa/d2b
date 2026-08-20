use d2b_contracts_provider::v3::{ProviderManifest, SemanticFamily};

#[test]
fn provider_contract_surface_exposes_strict_schema_types() {
    let _ = std::any::type_name::<ProviderManifest>();
    assert_eq!(SemanticFamily::Audio.namespace(), "audio.d2bus.org");
}
