use std::sync::Mutex;

use d2b_contracts::v3::{ResourceRef, ZoneId};
use d2b_provider_supervisor::{
    NotificationHostSinkIdentity, NotificationLifecycleBackend, NotificationLifecycleObservation,
    NotificationLifecyclePlan, NotificationLifecycleSupervisor, NotificationSourceIdentity,
};

#[derive(Default)]
struct Backend {
    sources: Mutex<Vec<NotificationSourceIdentity>>,
    sink: Mutex<Option<NotificationHostSinkIdentity>>,
    fail_source_start_once: std::sync::Arc<Mutex<bool>>,
}

impl NotificationLifecycleBackend for Backend {
    fn start_source(&self, source: &NotificationSourceIdentity) -> Result<(), &'static str> {
        let mut fail = self.fail_source_start_once.lock().unwrap();
        if *fail {
            *fail = false;
            return Err("source-start-failed");
        }
        self.sources.lock().unwrap().push(source.clone());
        Ok(())
    }

    fn stop_source(&self, source: &NotificationSourceIdentity) -> Result<(), &'static str> {
        self.sources
            .lock()
            .unwrap()
            .retain(|active| active != source);
        Ok(())
    }

    fn start_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), &'static str> {
        *self.sink.lock().unwrap() = Some(sink.clone());
        Ok(())
    }

    fn stop_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), &'static str> {
        let mut active = self.sink.lock().unwrap();
        if active.as_ref() == Some(sink) {
            *active = None;
            Ok(())
        } else {
            Err("host-sink-not-active")
        }
    }

    fn observe(
        &self,
        _zone: &ZoneId,
        _provider_ref: &ResourceRef,
    ) -> Result<NotificationLifecycleObservation, &'static str> {
        Ok(NotificationLifecycleObservation::new(
            self.sources.lock().unwrap().clone(),
            self.sink.lock().unwrap().clone(),
        ))
    }
}

fn plan() -> NotificationLifecyclePlan {
    let zone = ZoneId::parse("work").unwrap();
    let provider = ResourceRef::parse("Provider/notification-desktop").unwrap();
    let source = NotificationSourceIdentity::new(
        zone.clone(),
        provider.clone(),
        ResourceRef::parse("Guest/guest").unwrap(),
        3,
        5,
        "sha256:source",
    )
    .unwrap();
    let sink = NotificationHostSinkIdentity::new(
        zone,
        provider,
        ResourceRef::parse("Host/host").unwrap(),
        ResourceRef::parse("User/alice").unwrap(),
        ResourceRef::parse("Provider/display-wayland").unwrap(),
        5,
        7,
    )
    .unwrap();
    NotificationLifecyclePlan::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse("Provider/notification-desktop").unwrap(),
        vec![source],
        Vec::new(),
        Some(sink),
        None,
    )
    .unwrap()
}

#[test]
fn supervisor_issues_only_complete_generation_bound_receipts() {
    let supervisor = NotificationLifecycleSupervisor::new(Backend::default());
    let plan = plan();

    let receipt = supervisor.apply(&plan).unwrap();

    assert!(receipt.matches(&plan));
    assert_eq!(
        supervisor
            .recover(plan.zone(), plan.provider_ref())
            .unwrap(),
        2
    );
}

#[test]
fn supervisor_rolls_back_partial_effects_for_retry() {
    let backend = Backend::default();
    let fail_source_start_once = backend.fail_source_start_once.clone();
    let supervisor = NotificationLifecycleSupervisor::new(backend);
    let first = plan();
    supervisor.apply(&first).unwrap();

    let replacement = NotificationSourceIdentity::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse("Provider/notification-desktop").unwrap(),
        ResourceRef::parse("Guest/replacement").unwrap(),
        4,
        5,
        "sha256:replacement",
    )
    .unwrap();
    let transition = NotificationLifecyclePlan::new(
        ZoneId::parse("work").unwrap(),
        ResourceRef::parse("Provider/notification-desktop").unwrap(),
        vec![replacement],
        first.start_sources().to_vec(),
        None,
        first.start_host_sink().cloned(),
    )
    .unwrap();
    *fail_source_start_once.lock().unwrap() = true;

    assert!(matches!(
        supervisor.apply(&transition),
        Err("source-start-failed")
    ));
    assert!(supervisor.apply(&transition).is_ok());
    assert!(!supervisor.is_drained().unwrap());
}
