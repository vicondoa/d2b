use d2b_contracts::v3::{ResourcePhase, ZoneHandlerName, ZoneHandlerPhase, ZoneHandlerStatus};
use d2b_core_controller::zone_status::{SystemCoreStatusEmitter, ZoneStatusInput};

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
