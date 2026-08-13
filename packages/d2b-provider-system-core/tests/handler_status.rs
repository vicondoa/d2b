use d2b_contracts::v3::{ZoneHandlerName, ZoneHandlerPhase, ZoneHandlerStatus};
use d2b_provider_system_core::{
    HandlerReadinessError, SYSTEM_CORE_HOST_HANDLER, SYSTEM_CORE_USER_HANDLER, emit_handler_status,
    require_ready_handlers,
};

#[test]
fn emitter_uses_only_hyphenated_system_core_handler_values() {
    let handlers = emit_handler_status(ZoneHandlerPhase::Ready, ZoneHandlerPhase::Ready, None);
    assert_eq!(handlers.len(), 2);
    assert_eq!(handlers[0].name(), ZoneHandlerName::SystemCoreHost);
    assert_eq!(handlers[1].name(), ZoneHandlerName::SystemCoreUser);
    assert_eq!(
        serde_json::to_string(&handlers[0].name()).unwrap(),
        format!("\"{SYSTEM_CORE_HOST_HANDLER}\"")
    );
    assert_eq!(
        serde_json::to_string(&handlers[1].name()).unwrap(),
        format!("\"{SYSTEM_CORE_USER_HANDLER}\"")
    );
    let pending = emit_handler_status(ZoneHandlerPhase::Pending, ZoneHandlerPhase::Degraded, None);
    assert_eq!(pending[0].phase(), ZoneHandlerPhase::Pending);
    assert_eq!(pending[1].phase(), ZoneHandlerPhase::Degraded);
    assert_eq!(
        require_ready_handlers(&pending),
        Err(HandlerReadinessError::NotReady)
    );
}

#[test]
fn missing_duplicate_and_non_ready_pairs_fail_closed() {
    let host = ZoneHandlerStatus::new(
        ZoneHandlerName::SystemCoreHost,
        ZoneHandlerPhase::Ready,
        None,
    );
    let user = ZoneHandlerStatus::new(
        ZoneHandlerName::SystemCoreUser,
        ZoneHandlerPhase::Ready,
        None,
    );
    assert_eq!(
        require_ready_handlers(std::slice::from_ref(&host)),
        Err(HandlerReadinessError::PairInvalid)
    );
    assert_eq!(
        require_ready_handlers(&[host.clone(), host, user.clone(),]),
        Err(HandlerReadinessError::PairInvalid)
    );
    assert_eq!(
        require_ready_handlers(&[
            ZoneHandlerStatus::new(
                ZoneHandlerName::SystemCoreHost,
                ZoneHandlerPhase::Pending,
                None,
            ),
            user,
        ]),
        Err(HandlerReadinessError::NotReady)
    );
}

#[test]
fn provider_lifecycle_cannot_substitute_for_system_core_pair() {
    let handlers = vec![ZoneHandlerStatus::new(
        ZoneHandlerName::ProviderLifecycle,
        ZoneHandlerPhase::Ready,
        None,
    )];
    assert_eq!(
        require_ready_handlers(&handlers),
        Err(HandlerReadinessError::PairInvalid)
    );
}
