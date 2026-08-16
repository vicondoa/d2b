use d2b_provider_system_core::testing::{ScriptedDiscoveryPort, block_on, fixtures};
use d2b_provider_system_core::{HostReconciler, UserDiscoveryCondition, UserReconciler};

#[test]
fn host_and_user_handlers_emit_redacted_reconcile_audits() {
    let host_ref = fixtures::host_ref();
    let host = HostReconciler::new()
        .reconcile_with_audit(
            &host_ref,
            &fixtures::system_core_provider_ref(),
            &fixtures::system_host_spec(),
        )
        .unwrap();
    assert_eq!(host.1.handler, "system-core-host");
    assert!(
        !serde_json::to_string(&host.1)
            .unwrap()
            .contains("host-system")
    );

    let user_ref = fixtures::user_ref();
    let user = block_on(
        UserReconciler::new(ScriptedDiscoveryPort::resolving([
            d2b_provider_system_core::UserBinding::NssRecord,
            d2b_provider_system_core::UserBinding::PrimaryGroup,
        ]))
        .reconcile_with_audit(&user_ref, &fixtures::user_spec()),
    )
    .unwrap();
    assert_eq!(user.0.discovery, UserDiscoveryCondition::Discovered);
    assert_eq!(user.1.handler, "system-core-user");
}
