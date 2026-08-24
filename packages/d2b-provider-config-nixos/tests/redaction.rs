use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_config_nixos::{ConfigService, GuestConfigDocument, GuestSessionEvidence};

#[test]
fn debug_does_not_include_guest_content_or_boot_identity() {
    let document = GuestConfigDocument::new(b"secret = true;".to_vec()).expect("document");
    let document_debug = format!("{document:?}");
    assert!(!document_debug.contains("secret"));

    let evidence = GuestSessionEvidence::new(
        ResourceRef::parse("Guest/work").expect("guest ref"),
        "private-boot-id",
        1,
    ).expect("evidence");
    let evidence_debug = format!("{evidence:?}");
    assert!(!evidence_debug.contains("private-boot-id"));
    let _ = ConfigService;
}
