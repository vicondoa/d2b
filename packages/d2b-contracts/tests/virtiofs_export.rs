use d2b_contracts::v3::{
    ResourceRef, VIRTIOFS_EXPORT_RESOURCE_TYPE, VirtiofsExportSpec, volume::AttachmentAccess,
};

#[test]
fn export_spec_is_strict_and_carries_only_typed_refs() {
    let spec = VirtiofsExportSpec::new(
        ResourceRef::parse("Provider/volume-virtiofs").unwrap(),
        ResourceRef::parse("Volume/store-view").unwrap(),
        ResourceRef::parse("Guest/work-vm").unwrap(),
        "ro-store",
        AttachmentAccess::ReadOnly,
        "/nix/.ro-store",
    )
    .unwrap();
    assert_eq!(spec.resource_type(), VIRTIOFS_EXPORT_RESOURCE_TYPE);
    assert_eq!(spec.volume_ref().to_canonical_string(), "Volume/store-view");
    assert_eq!(spec.execution_ref().to_canonical_string(), "Guest/work-vm");
    assert!(serde_json::from_str::<VirtiofsExportSpec>(
        r#"{"providerRef":"Provider/volume-virtiofs","volumeRef":"Volume/store-view","executionRef":"Guest/work-vm","view":"ro-store","access":"read-only","mountPath":"/nix/.ro-store","hostPath":"/nix/store"}"#
    )
    .is_err());
}

#[test]
fn export_spec_rejects_noncanonical_provider_identity() {
    assert!(
        VirtiofsExportSpec::new(
            ResourceRef::parse("Provider/other").unwrap(),
            ResourceRef::parse("Volume/store-view").unwrap(),
            ResourceRef::parse("Guest/work-vm").unwrap(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            "/nix/.ro-store",
        )
        .is_err()
    );
}
