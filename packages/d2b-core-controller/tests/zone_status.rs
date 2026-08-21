use d2b_contracts_zone_session::v3::{
    ZoneHandlerName,
    ZoneHandlerPhase,
    ZoneHandlerStatus,
};
use d2b_contracts_resource::v3::{
    ResourcePhase,
    Timestamp,
};
use d2b_core_controller::zone_status::{
    SystemCoreStatusEmitter, ZoneRuntimeMetadata, ZoneStatusInput,
};

#[test]
fn production_emitter_always_includes_exact_system_core_pair() {
    let input = ZoneStatusInput::new(
        ResourcePhase::Ready,
        vec![ZoneHandlerStatus::new(
            ZoneHandlerName::ProviderLifecycle,
            ZoneHandlerPhase::Ready,
            None,
        )],
    );
    let status = SystemCoreStatusEmitter::new().emit(input).unwrap();
    assert_eq!(
        status
            .handlers()
            .iter()
            .filter(|handler| handler.name() == ZoneHandlerName::SystemCoreHost)
            .count(),
        1
    );
    assert_eq!(
        status
            .handlers()
            .iter()
            .filter(|handler| handler.name() == ZoneHandlerName::SystemCoreUser)
            .count(),
        1
    );
    assert!(status.mandatory_handlers_ready());
}

#[test]
fn malformed_system_core_input_cannot_publish_ready_status() {
    let input = ZoneStatusInput::new(
        ResourcePhase::Ready,
        vec![ZoneHandlerStatus::new(
            ZoneHandlerName::SystemCoreHost,
            ZoneHandlerPhase::Failed,
            None,
        )],
    );
    let status = SystemCoreStatusEmitter::new().emit(input).unwrap();
    assert!(!status.mandatory_handlers_ready());
}

#[test]
fn duplicate_system_core_handler_records_are_rejected() {
    let input = ZoneStatusInput::new(
        ResourcePhase::Ready,
        vec![
            ZoneHandlerStatus::new(
                ZoneHandlerName::SystemCoreHost,
                ZoneHandlerPhase::Ready,
                None,
            ),
            ZoneHandlerStatus::new(
                ZoneHandlerName::SystemCoreHost,
                ZoneHandlerPhase::Ready,
                None,
            ),
        ],
    );
    assert!(SystemCoreStatusEmitter::new().emit(input).is_err());
}

#[test]
fn runtime_metadata_and_reconcile_timestamp_are_projected() {
    let timestamp = Timestamp::parse("2026-08-15T04:10:30.123Z").unwrap();
    let input = ZoneStatusInput::new(ResourcePhase::Ready, Vec::new()).with_runtime_metadata(
        ZoneRuntimeMetadata {
            api_catalog_revision: 7,
            policy_revision: 11,
            configuration_revision: 13,
            installed_provider_count: 3,
            ready_provider_count: 2,
            total_resource_count: 9,
            active_configuration_generation: 17,
            generation_cleanup_pending: true,
            cleanup_pending_count: 1,
            last_reconciled_at: Some(timestamp.clone()),
        },
    );

    let status = SystemCoreStatusEmitter::new().emit(input).unwrap();
    assert_eq!(status.api_catalog_revision(), 7);
    assert_eq!(status.policy_revision(), 11);
    assert_eq!(status.configuration_revision(), 13);
    assert_eq!(status.installed_provider_count(), 3);
    assert_eq!(status.ready_provider_count(), 2);
    assert_eq!(status.total_resource_count(), 9);
    assert_eq!(
        status
            .handlers()
            .iter()
            .find(|handler| handler.name() == ZoneHandlerName::SystemCoreHost)
            .and_then(ZoneHandlerStatus::last_reconciled_at),
        Some(&timestamp)
    );
}
