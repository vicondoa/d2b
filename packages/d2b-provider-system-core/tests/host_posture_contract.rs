#[path = "../src/host_process_audit.rs"]
mod host_process_audit;
#[path = "../src/host_reconciler.rs"]
mod host_reconciler;
#[path = "../src/host_status.rs"]
mod host_status;

use d2b_contracts::v3::{ResourceRef, host::HostSpec};

#[test]
fn status_projection_declares_user_only_posture() {
    let user = ResourceRef::parse("User/alice").unwrap();
    let status = host_reconciler::reconcile_status(&HostSpec::user_only(user).unwrap());
    assert!(status.is_no_isolation());
    assert_eq!(
        host_reconciler::isolation_posture(
            &HostSpec::user_only(ResourceRef::parse("User/alice").unwrap()).unwrap()
        ),
        Some(d2b_contracts::v3::host::IsolationPosture::NoIsolation)
    );
    assert_eq!(
        status.isolation_posture,
        Some(d2b_contracts::v3::host::IsolationPosture::NoIsolation)
    );
    assert_eq!(
        status.isolation_posture_message,
        Some(host_status::ISOLATION_POSTURE_MESSAGE)
    );
}

#[test]
fn operator_cannot_supply_or_clear_posture() {
    for value in [
        serde_json::json!({"isolationPosture": "none"}),
        serde_json::json!({"isolationPosture": null}),
        serde_json::json!({"isolationPostureMessage": "suppressed"}),
    ] {
        assert_eq!(
            host_reconciler::reject_operator_status_fields(&value),
            Err(host_reconciler::HostStatusInputError::OperatorSuppliedField)
        );
    }
    assert_eq!(
        host_reconciler::reject_operator_status_fields(&serde_json::json!({"phase": "Ready"})),
        Ok(())
    );
}

#[test]
fn posture_projection_debug_is_redacted() {
    let status = host_reconciler::reconcile_status(
        &HostSpec::user_only(ResourceRef::parse("User/alice").unwrap()).unwrap(),
    );
    assert_eq!(format!("{status:?}"), "HostStatusProjection(<redacted>)");
}

#[test]
fn host_posture_contract_emits_redacted_launch_and_stop_effects() {
    let mut effects = Vec::new();
    struct Port<'a>(&'a mut Vec<host_process_audit::ProcessEffectFields>);
    impl host_process_audit::HostProcessAuditPort for Port<'_> {
        type Error = std::convert::Infallible;

        fn append_process_effect(
            &mut self,
            effect: host_process_audit::ProcessEffectFields,
        ) -> Result<(), Self::Error> {
            self.0.push(effect);
            Ok(())
        }
    }

    let canary = "opaque-process-canary";
    host_process_audit::emit_launch(
        &mut Port(&mut effects),
        host_process_audit::ProcessEffectDomain::User,
        true,
        format!("sha256:{canary}"),
        canary,
        host_process_audit::ProcessEffectOutcome::Ok,
    )
    .unwrap();
    host_process_audit::emit_stop(
        &mut Port(&mut effects),
        host_process_audit::ProcessEffectDomain::User,
        true,
        format!("sha256:{canary}"),
        canary,
        host_process_audit::ProcessEffectOutcome::Ok,
        Some(host_process_audit::ProcessExitClass::Exited),
    )
    .unwrap();

    assert_eq!(effects.len(), 2);
    assert_eq!(
        effects[0].event(),
        host_process_audit::ProcessEffectEvent::Launch
    );
    assert_eq!(
        effects[1].event(),
        host_process_audit::ProcessEffectEvent::Stop
    );
    assert!(effects.iter().all(|effect| effect.no_isolation()));
    assert!(
        !format!("{:?}", effects[0]).contains(canary),
        "opaque process identity must not appear in diagnostics"
    );
}
