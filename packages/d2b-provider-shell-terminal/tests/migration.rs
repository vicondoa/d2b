use d2b_provider_shell_terminal::{MigrationDisposition, ProviderStateSet};

#[test]
fn provider_has_no_persistent_state_or_legacy_protocol_surface() {
    assert_eq!(ProviderStateSet::canonical(), ProviderStateSet::Empty);
    assert_eq!(
        MigrationDisposition::canonical(),
        MigrationDisposition::NoProviderLegacyProtocol
    );
}
