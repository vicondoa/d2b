use std::sync::Mutex;

use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_provider_notification_desktop::{
    NotificationHostSinkIdentity, NotificationLifecycleBackend, NotificationLifecycleObservation,
    NotificationLifecyclePlan, NotificationLifecycleSupervisor, NotificationSourceIdentity,
    ProviderError,
};

#[derive(Default)]
struct Backend {
    sources: Mutex<Vec<NotificationSourceIdentity>>,
    sink: Mutex<Option<NotificationHostSinkIdentity>>,
    fail_source_start_once: std::sync::Arc<Mutex<bool>>,
}

impl NotificationLifecycleBackend for Backend {
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn start_source(&self, source: &NotificationSourceIdentity) -> Result<(), ProviderError> {
        let mut fail = self.fail_source_start_once.lock().unwrap();
        if *fail {
            *fail = false;
            return Err(ProviderError::LifecycleSourceStartFailed);
        }
        self.sources.lock().unwrap().push(source.clone());
        Ok(())
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn stop_source(&self, source: &NotificationSourceIdentity) -> Result<(), ProviderError> {
        self.sources
            .lock()
            .unwrap()
            .retain(|active| active != source);
        Ok(())
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn start_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), ProviderError> {
        *self.sink.lock().unwrap() = Some(sink.clone());
        Ok(())
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn stop_host_sink(&self, sink: &NotificationHostSinkIdentity) -> Result<(), ProviderError> {
        let mut active = self.sink.lock().unwrap();
        if active.as_ref() == Some(sink) {
            *active = None;
            Ok(())
        } else {
            Err(ProviderError::HostSinkLifecycleMismatch)
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn observe(
        &self,
        _zone: &ZoneId,
        _provider_ref: &ResourceRef,
    ) -> Result<NotificationLifecycleObservation, ProviderError> {
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
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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
        Err(ProviderError::LifecycleSourceStartFailed)
    ));
    assert!(supervisor.apply(&transition).is_ok());
    assert!(!supervisor.is_drained().unwrap());
}

// --- U28: declared endpoint authority over the host-sink transition ---

use d2b_contracts_resource::v3::{
    DesiredRevision, EndpointAttachmentKind, ResourceGeneration, ResourceUid, StoreIncarnation,
    ZoneDesiredSequence, identity::ReconnectGeneration,
};
use d2b_provider_notification_desktop::{
    NOTIFICATION_SERVICE, NotificationEndpointBinding, NotificationEndpointEvidence,
    NotificationEndpointFence, NotificationEndpointGate, NotificationEndpointRole,
    NotificationHostEndpoints, admit_notification_endpoint, notification_endpoint_bindings,
};

const SOURCE_ENDPOINT: &str = "Endpoint/notification-guest-source";
const DESKTOP_ENDPOINT: &str = "Endpoint/notification-desktop-sink";
const SINK_CONSUMER: &str = "Process/notification-sink";

fn endpoints() -> NotificationHostEndpoints {
    NotificationHostEndpoints::new(
        ResourceRef::parse(SOURCE_ENDPOINT).expect("endpoint"),
        ResourceRef::parse(DESKTOP_ENDPOINT).expect("endpoint"),
    )
    .expect("declared endpoints")
}

fn sink_consumer() -> ResourceRef {
    ResourceRef::parse(SINK_CONSUMER).expect("consumer")
}

fn fence() -> NotificationEndpointFence {
    NotificationEndpointFence::new(
        ZoneId::parse("work").expect("zone"),
        StoreIncarnation::parse("store-one").expect("store"),
        DesiredRevision::INITIAL.try_next().expect("revision"),
        ZoneDesiredSequence::INITIAL.try_next().expect("sequence"),
        ResourceGeneration::new(5).expect("source generation"),
        ResourceGeneration::new(7).expect("consumer generation"),
        ReconnectGeneration::new(2).expect("reconnect"),
    )
}

fn evidence(fence: &NotificationEndpointFence) -> NotificationEndpointEvidence {
    NotificationEndpointEvidence {
        zone: fence.zone().clone(),
        store: fence.store().clone(),
        source_generation: fence.source_generation(),
        consumer_generation: fence.consumer_generation(),
        desired_revision: fence.desired_revision(),
        sequence: fence.sequence(),
        reconnect: ReconnectGeneration::new(9).expect("reconnect"),
    }
}

fn binding(role: NotificationEndpointRole) -> NotificationEndpointBinding {
    notification_endpoint_bindings(&endpoints(), &sink_consumer())
        .expect("bindings")
        .into_iter()
        .find(|declared| declared.role() == role)
        .expect("declared binding")
}

/// The committed rows one gate is evaluated against.
struct Env {
    endpoints: NotificationHostEndpoints,
    consumer: ResourceRef,
    source_uid: ResourceUid,
    consumer_uid: ResourceUid,
}

fn env() -> Env {
    Env {
        endpoints: endpoints(),
        consumer: sink_consumer(),
        source_uid: ResourceUid::parse("bbbbbbbb-0000-4000-8000-000000000001").expect("uid"),
        consumer_uid: ResourceUid::parse("bbbbbbbb-0000-4000-8000-000000000002").expect("uid"),
    }
}

impl Env {
    fn gate<'a>(
        &'a self,
        binding: &'a NotificationEndpointBinding,
        fence: &'a NotificationEndpointFence,
        evidence: &'a NotificationEndpointEvidence,
    ) -> NotificationEndpointGate<'a> {
        NotificationEndpointGate {
            endpoints: &self.endpoints,
            consumer: &self.consumer,
            binding,
            fence,
            evidence,
            source_uid: &self.source_uid,
            consumer_uid: &self.consumer_uid,
        }
    }
}

#[test]
fn declared_service_is_the_one_source_of_notification_methods_and_streams() {
    assert_eq!(NOTIFICATION_SERVICE.id, "d2b.notification.v3");
    assert_eq!(
        NOTIFICATION_SERVICE.streams,
        &["DesktopNotificationSink", "DesktopNotificationObserver"]
    );
    for role in NotificationEndpointRole::ALL {
        assert!(NOTIFICATION_SERVICE.streams.contains(&role.stream()));
    }
    // Every declared method name is unique, so a purpose can never be served
    // by two methods.
    let names: Vec<&str> = NOTIFICATION_SERVICE
        .methods
        .iter()
        .map(|method| method.name)
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
    assert!(NOTIFICATION_SERVICE.declares_method("deliver-source"));
    assert!(NOTIFICATION_SERVICE.declares_method("invoke-action"));
    assert!(!NOTIFICATION_SERVICE.declares_method("capture-host-selection"));
}

#[test]
fn host_sink_transition_requires_an_admitted_presentation_endpoint() {
    let backend = Backend::default();
    let supervisor = NotificationLifecycleSupervisor::new(backend);
    let live = plan();

    let desktop = binding(NotificationEndpointRole::DesktopSink);
    let committed = fence();
    let observed = evidence(&committed);
    let committed_rows = env();
    let admitted = committed_rows.gate(&desktop, &committed, &observed);

    let receipt = supervisor.apply_over_endpoint(&live, &admitted).expect("applied");

    assert!(receipt.matches(&live));
}

#[test]
fn a_withdrawn_presentation_endpoint_refuses_the_host_sink_transition() {
    let backend = Backend::default();
    let supervisor = NotificationLifecycleSupervisor::new(backend);

    let desktop = binding(NotificationEndpointRole::DesktopSink);
    let revoked = fence().revoke();
    let observed = evidence(&revoked);
    let withdrawn_rows = env();
    let withdrawn = withdrawn_rows.gate(&desktop, &revoked, &observed);

    assert_eq!(
        supervisor
            .apply_over_endpoint(&plan(), &withdrawn)
            .err()
            .map(|error| error.to_string()),
        Some("notification-host-sink-endpoint-unauthenticated".to_owned())
    );
    // No host effect ran: the sink was never started on another channel.
    assert!(supervisor.is_drained().expect("drained"));

    let draining = fence().drain();
    let draining_observed = evidence(&draining);
    let draining_rows = env();
    let draining_gate = draining_rows.gate(&desktop, &draining, &draining_observed);
    assert_eq!(
        supervisor
            .apply_over_endpoint(&plan(), &draining_gate)
            .err()
            .map(|error| error.to_string()),
        Some("notification-host-sink-endpoint-unauthenticated".to_owned())
    );
    assert!(supervisor.is_drained().expect("drained"));
}

#[test]
fn a_guest_source_relationship_cannot_authorize_the_host_sink_transition() {
    let supervisor = NotificationLifecycleSupervisor::new(Backend::default());
    let guest = binding(NotificationEndpointRole::GuestSource);
    let committed = fence();
    let observed = evidence(&committed);
    let committed_rows = env();
    let wrong_channel = committed_rows.gate(&guest, &committed, &observed);

    assert_eq!(
        supervisor
            .apply_over_endpoint(&plan(), &wrong_channel)
            .err()
            .map(|error| error.to_string()),
        Some("notification-host-sink-endpoint-unauthenticated".to_owned())
    );
    assert!(supervisor.is_drained().expect("drained"));
}

#[test]
fn source_and_sink_identities_derive_their_relationships_from_committed_rows() {
    let source = NotificationSourceIdentity::new(
        ZoneId::parse("work").expect("zone"),
        ResourceRef::parse("Provider/notification-desktop").expect("provider"),
        ResourceRef::parse("Guest/guest").expect("guest"),
        3,
        5,
        "sha256:source",
    )
    .expect("source");

    // The caller's endpoint label is not an input: a label that names another
    // Provider, another slot, or another purpose derives the same
    // relationship as a label that names nothing.
    let relabelled = NotificationSourceIdentity::new(
        ZoneId::parse("work").expect("zone"),
        ResourceRef::parse("Provider/notification-desktop").expect("provider"),
        ResourceRef::parse("Guest/guest").expect("guest"),
        3,
        5,
        NotificationEndpointRole::DesktopSink.slot(),
    )
    .expect("source");
    assert_eq!(
        source
            .source_binding(&endpoints(), &sink_consumer())
            .expect("source binding"),
        relabelled
            .source_binding(&endpoints(), &sink_consumer())
            .expect("source binding")
    );

    let sink = NotificationHostSinkIdentity::new(
        ZoneId::parse("work").expect("zone"),
        ResourceRef::parse("Provider/notification-desktop").expect("provider"),
        ResourceRef::parse("Host/host").expect("host"),
        ResourceRef::parse("User/alice").expect("user"),
        ResourceRef::parse("Provider/display-wayland").expect("display"),
        5,
        7,
    )
    .expect("sink");
    let sink_binding = sink
        .desktop_sink_binding(&endpoints(), &sink_consumer())
        .expect("sink binding");
    assert_eq!(sink_binding.role(), NotificationEndpointRole::DesktopSink);
    assert_eq!(
        sink_binding.request().attachment(),
        EndpointAttachmentKind::Connect
    );
    assert_eq!(
        sink_binding.request().purpose().as_str(),
        NotificationEndpointRole::DesktopSink.purpose()
    );

    let committed = fence();
    let observed = evidence(&committed);
    let committed_rows = env();
    let admitted = committed_rows.gate(&sink_binding, &committed, &observed);
    let authorized = sink.admit_host_sink_transition(&admitted);
    assert!(authorized.is_ok());
    assert_eq!(
        authorized.expect("admitted").role(),
        NotificationEndpointRole::DesktopSink
    );

    // A relationship over the guest-source endpoint is refused rather than
    // repaired into the one the transition needs.
    let forged = NotificationHostEndpoints::new(
        ResourceRef::parse(DESKTOP_ENDPOINT).expect("endpoint"),
        ResourceRef::parse(SOURCE_ENDPOINT).expect("endpoint"),
    )
    .expect("endpoints");
    let forged_binding = sink
        .desktop_sink_binding(&forged, &sink_consumer())
        .expect("forged binding");
    let forged_gate = committed_rows.gate(&forged_binding, &committed, &observed);
    assert_eq!(
        admit_notification_endpoint(&forged_gate)
            .map_err(|refusal| refusal.code())
            .expect_err("forged relationship"),
        "notification-endpoint-request-mismatch"
    );
    assert_eq!(
        sink.admit_host_sink_transition(&forged_gate)
            .err()
            .map(|error| error.to_string()),
        Some("notification-host-sink-endpoint-unauthenticated".to_owned())
    );
}
