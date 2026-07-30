//! Hermetic proofs for the v3 Provider registry, its ZonePath-keyed session
//! identity, and forwarding admission.

use std::time::Duration;

use d2b_contracts::v3::{
    identity::{
        AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, EvidenceClass,
        Locality, ReconnectGeneration, ResourceGeneration, ResourceName, ResourceTypeName,
        ResourceUid, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
        TranscriptHash, TransportBinding,
    },
    resource_ref::ResourceRef,
    zone_routing::{ZoneLabelId, ZonePath},
};
use d2b_provider::{
    AdmissionOptions, CancellationToken, ForwardTarget, LocalHopGrants, PROVIDER_SCHEMA_VERSION,
    ProviderCapabilitySet, ProviderClass, ProviderDescriptor, ProviderForwardRequest,
    ProviderImplementationId, ProviderMethodName, ProviderRegistry, ProviderRegistryBuilder,
    ProviderRegistryManager, ProviderRuntimeError, RegistryBuildError, RegistryDrainPolicy,
    RegistryLifecycle, RegistryLimits, SessionIdentity, ZoneRouteFailClosedReason,
    admit_provider_forward,
};

const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
const UID: &str = "123e4567-e89b-42d3-a456-426614174000";

fn zone(labels: &[&str]) -> ZonePath {
    ZonePath::new(
        labels
            .iter()
            .map(|label| ZoneLabelId::parse(*label).expect("valid zone label"))
            .collect(),
    )
    .expect("valid zone path")
}

fn provider_ref(name: &str) -> ResourceRef {
    ResourceRef::parse(&format!("Provider/{name}")).expect("valid Provider ref")
}

fn service() -> ServiceName {
    ServiceName::parse("d2b.provider.v3").expect("valid service name")
}

fn generation(value: u64) -> ConfigurationGeneration {
    ConfigurationGeneration::new(value).expect("nonzero generation")
}

fn provider_generation(value: u64) -> ResourceGeneration {
    ResourceGeneration::new(value).expect("nonzero generation")
}

fn method(name: &str) -> ProviderMethodName {
    ProviderMethodName::parse(name).expect("valid method token")
}

fn capabilities(names: &[&str]) -> ProviderCapabilitySet {
    ProviderCapabilitySet::new(names.iter().map(|name| method(name))).expect("valid capabilities")
}

fn descriptor(
    zone_path: &ZonePath,
    name: &str,
    registry_generation: u64,
    methods: &[&str],
) -> ProviderDescriptor {
    ProviderDescriptor::new(
        zone_path.clone(),
        provider_ref(name),
        ProviderClass::Runtime,
        ProviderImplementationId::parse("runtime-fake").expect("valid implementation token"),
        generation(registry_generation),
        provider_generation(7),
        service(),
        capabilities(methods),
    )
    .expect("valid descriptor")
}

/// Build the authenticated evidence a Zone runtime would have established.
fn subject(provider: Option<&str>, service_name: ServiceName) -> AuthenticatedSubjectContext {
    let binding = SessionBinding::new(
        SchemaFingerprint::parse(DIGEST).expect("valid fingerprint"),
        TransportBinding::new(
            Locality::Local,
            BindingDigest::parse(DIGEST).expect("valid digest"),
        ),
        ReconnectGeneration::new(1).expect("nonzero reconnect generation"),
        TranscriptHash::from_bytes([0u8; 32]),
    );
    let context = AuthenticatedSubjectContext::new(
        ResourceRef::parse("Process/caller").expect("valid subject ref"),
        ResourceUid::parse(UID).expect("valid uid"),
        ResourceRef::parse("Zone/work").expect("valid zone ref"),
        EvidenceClass::UnixPeer,
        SessionPurpose::parse("provider-invoke").expect("valid purpose"),
        service_name,
        binding,
    );
    match provider {
        Some(name) => context
            .with_provider_ref(provider_ref(name))
            .with_provider_generation(provider_generation(7)),
        None => context,
    }
}

fn identity(zone_path: &ZonePath, provider: &str) -> SessionIdentity {
    SessionIdentity::from_authenticated(zone_path.clone(), &subject(Some(provider), service()))
        .expect("authenticated evidence carries the provider binding")
}

fn registry(zone_path: &ZonePath, gen_value: u64) -> ProviderRegistry<&'static str> {
    let mut builder = ProviderRegistryBuilder::new(zone_path.clone(), generation(gen_value));
    builder
        .register_instance(
            descriptor(zone_path, "runtime-a", gen_value, &["start", "stop"]),
            "runtime-a-instance",
        )
        .expect("descriptor registers");
    builder.finish().expect("registry seals")
}

fn admission(zone_path: &ZonePath, provider: &str, requested: &str) -> AdmissionOptions {
    AdmissionOptions {
        identity: identity(zone_path, provider),
        expected_method: method(requested),
        deadline_after: Duration::from_secs(2),
        caller_cancellation: CancellationToken::new(),
    }
}

fn drain_policy() -> RegistryDrainPolicy {
    RegistryDrainPolicy {
        drain_deadline_ms: 100,
        cancel_in_flight_at_deadline: true,
        close_provider_sessions: true,
    }
}

#[test]
fn the_descriptor_publishes_the_v3_schema_version() {
    assert_eq!(PROVIDER_SCHEMA_VERSION, 3);
    let work = zone(&["work"]);
    assert_eq!(
        descriptor(&work, "runtime-a", 1, &["start"]).schema_version(),
        PROVIDER_SCHEMA_VERSION
    );
}

#[test]
fn the_eleven_provider_families_are_preserved() {
    assert_eq!(ProviderClass::ALL.len(), 11);
    let mut seen: Vec<&str> = ProviderClass::ALL.iter().map(|c| c.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 11);
}

#[test]
fn registry_limits_reject_a_zero_cap_and_a_per_provider_cap_above_the_total() {
    assert_eq!(
        RegistryLimits {
            total_in_flight: 0,
            per_provider_in_flight: 1,
        }
        .validate(),
        Err(RegistryBuildError::BoundExceeded)
    );
    assert_eq!(
        RegistryLimits {
            total_in_flight: 4,
            per_provider_in_flight: 0,
        }
        .validate(),
        Err(RegistryBuildError::BoundExceeded)
    );
    assert_eq!(
        RegistryLimits {
            total_in_flight: 4,
            per_provider_in_flight: 5,
        }
        .validate(),
        Err(RegistryBuildError::BoundExceeded)
    );
    let ok = RegistryLimits {
        total_in_flight: 4,
        per_provider_in_flight: 4,
    };
    assert_eq!(ok.validate(), Ok(ok));
    let default = RegistryLimits::default();
    assert_eq!(default.total_in_flight, 256);
    assert_eq!(default.per_provider_in_flight, 32);
}

#[test]
fn a_generation_with_no_instance_does_not_seal() {
    let work = zone(&["work"]);
    let builder: ProviderRegistryBuilder<&'static str> =
        ProviderRegistryBuilder::new(work, generation(1));
    assert_eq!(
        builder.finish().err(),
        Some(RegistryBuildError::EmptyRegistry)
    );
}

#[test]
fn a_failed_step_aborts_the_whole_build_transaction() {
    let work = zone(&["work"]);
    let mut builder = ProviderRegistryBuilder::new(work.clone(), generation(1));
    assert_eq!(
        builder
            .register_instance(descriptor(&work, "runtime-a", 2, &["start"]), "a")
            .err(),
        Some(RegistryBuildError::GenerationMismatch)
    );
    assert_eq!(
        builder
            .register_instance(descriptor(&work, "runtime-b", 1, &["start"]), "b")
            .err(),
        Some(RegistryBuildError::TransactionAborted)
    );
    assert_eq!(
        builder.finish().err(),
        Some(RegistryBuildError::TransactionAborted)
    );
}

#[test]
fn a_descriptor_from_another_zone_is_refused() {
    let work = zone(&["work"]);
    let personal = zone(&["personal"]);
    let mut builder = ProviderRegistryBuilder::new(work, generation(1));
    assert_eq!(
        builder
            .register_instance(descriptor(&personal, "runtime-a", 1, &["start"]), "a")
            .err(),
        Some(RegistryBuildError::ZoneMismatch)
    );
}

#[test]
fn a_duplicate_provider_reference_is_refused() {
    let work = zone(&["work"]);
    let mut builder = ProviderRegistryBuilder::new(work.clone(), generation(1));
    builder
        .register_instance(descriptor(&work, "runtime-a", 1, &["start"]), "a")
        .expect("first registers");
    assert_eq!(
        builder
            .register_instance(descriptor(&work, "runtime-a", 1, &["start"]), "a-again")
            .err(),
        Some(RegistryBuildError::DuplicateProvider)
    );
}

#[test]
fn a_session_identity_requires_authenticated_provider_evidence() {
    let work = zone(&["work"]);
    assert_eq!(
        SessionIdentity::from_authenticated(work, &subject(None, service())).err(),
        Some(ProviderRuntimeError::MissingProviderBinding)
    );
}

// The v3 ZonePath routing proof: the identity is keyed by the Zone's path,
// and a registry admits only calls whose path is exactly its own.
#[test]
fn admission_requires_the_exact_zone_path() {
    let work = zone(&["work"]);
    let nested = zone(&["work", "payments"]);
    let registry = registry(&work, 1);

    let admitted = registry
        .admit(admission(&work, "runtime-a", "start"))
        .expect("the local Zone path is admitted");
    assert_eq!(admitted.instance, "runtime-a-instance");
    assert_eq!(admitted.context.identity().zone(), &work);
    drop(admitted);

    assert_eq!(
        registry
            .admit(admission(&nested, "runtime-a", "start"))
            .err(),
        Some(ProviderRuntimeError::SessionIdentityMismatch)
    );
}

#[test]
fn admission_requires_an_installed_provider_and_a_published_method() {
    let work = zone(&["work"]);
    let registry = registry(&work, 1);
    assert_eq!(
        registry.admit(admission(&work, "runtime-z", "start")).err(),
        Some(ProviderRuntimeError::UnknownProvider)
    );
    assert_eq!(
        registry.admit(admission(&work, "runtime-a", "drain")).err(),
        Some(ProviderRuntimeError::CapabilityDenied)
    );
}

#[test]
fn a_session_identity_from_another_service_does_not_match_the_descriptor() {
    let work = zone(&["work"]);
    let registry = registry(&work, 1);
    let other = ServiceName::parse("d2b.other.v3").expect("valid service name");
    let identity = SessionIdentity::from_authenticated(work, &subject(Some("runtime-a"), other))
        .expect("authenticated evidence carries the provider binding");
    let options = AdmissionOptions {
        identity,
        expected_method: method("start"),
        deadline_after: Duration::from_secs(2),
        caller_cancellation: CancellationToken::new(),
    };
    assert_eq!(
        registry.admit(options).err(),
        Some(ProviderRuntimeError::SessionIdentityMismatch)
    );
}

#[test]
fn the_in_flight_permit_is_released_when_the_admission_is_dropped() {
    let work = zone(&["work"]);
    let mut builder = ProviderRegistryBuilder::new(work.clone(), generation(1));
    builder
        .limits(RegistryLimits {
            total_in_flight: 1,
            per_provider_in_flight: 1,
        })
        .expect("valid limits");
    builder
        .register_instance(descriptor(&work, "runtime-a", 1, &["start"]), "a")
        .expect("descriptor registers");
    let registry = builder.finish().expect("registry seals");

    let held = registry
        .admit(admission(&work, "runtime-a", "start"))
        .expect("first admission");
    assert_eq!(
        registry.admit(admission(&work, "runtime-a", "start")).err(),
        Some(ProviderRuntimeError::InFlightLimit)
    );
    drop(held);
    registry
        .admit(admission(&work, "runtime-a", "start"))
        .expect("the permit was released on drop");
}

#[test]
fn an_invalid_drain_policy_is_refused() {
    for policy in [
        RegistryDrainPolicy {
            drain_deadline_ms: 0,
            cancel_in_flight_at_deadline: true,
            close_provider_sessions: true,
        },
        RegistryDrainPolicy {
            drain_deadline_ms: d2b_provider::MAX_REGISTRY_DRAIN_MS + 1,
            cancel_in_flight_at_deadline: true,
            close_provider_sessions: true,
        },
        RegistryDrainPolicy {
            drain_deadline_ms: 100,
            cancel_in_flight_at_deadline: false,
            close_provider_sessions: true,
        },
        RegistryDrainPolicy {
            drain_deadline_ms: 100,
            cancel_in_flight_at_deadline: true,
            close_provider_sessions: false,
        },
    ] {
        assert_eq!(
            policy.validate(),
            Err(ProviderRuntimeError::InvalidDrainPolicy)
        );
    }
}

#[tokio::test]
async fn shutdown_drains_then_retires_and_refuses_a_second_transition() {
    let work = zone(&["work"]);
    let registry = registry(&work, 1);
    assert_eq!(registry.lifecycle(), RegistryLifecycle::Accepting);

    let report = registry
        .shutdown(&drain_policy())
        .await
        .expect("shutdown succeeds");
    assert!(report.drained);
    assert_eq!(report.unresolved_in_flight, 0);
    assert_eq!(registry.lifecycle(), RegistryLifecycle::Retired);
    assert_eq!(registry.snapshot().lifecycle(), RegistryLifecycle::Retired);
    assert_eq!(
        registry.admit(admission(&work, "runtime-a", "start")).err(),
        Some(ProviderRuntimeError::NotAccepting)
    );
    assert_eq!(
        registry.shutdown(&drain_policy()).await.err(),
        Some(ProviderRuntimeError::InvalidLifecycleTransition)
    );
}

#[tokio::test]
async fn shutdown_reports_an_unresolved_call_and_cancels_its_context() {
    let work = zone(&["work"]);
    let registry = registry(&work, 1);
    let held = registry
        .admit(admission(&work, "runtime-a", "start"))
        .expect("admission");

    let report = registry
        .shutdown(&drain_policy())
        .await
        .expect("shutdown succeeds");
    assert!(!report.drained);
    assert_eq!(report.unresolved_in_flight, 1);
    assert!(held.context.is_cancelled());
    assert_eq!(
        held.context.remaining().err(),
        Some(ProviderRuntimeError::Cancelled)
    );
}

#[tokio::test]
async fn publish_swaps_the_generation_and_drains_the_outgoing_one() {
    let work = zone(&["work"]);
    let manager = ProviderRegistryManager::new(registry(&work, 1));
    let outgoing = manager.current();

    let report = manager
        .publish(registry(&work, 2), drain_policy())
        .await
        .expect("publish succeeds");
    assert!(report.drained);
    assert_eq!(outgoing.lifecycle(), RegistryLifecycle::Retired);
    assert_eq!(manager.current().snapshot().generation().get(), 2);
    manager
        .current()
        .admit(admission(&work, "runtime-a", "start"))
        .expect("the replacement generation admits");
}

#[tokio::test]
async fn publish_refuses_a_stale_generation_and_a_foreign_zone() {
    let work = zone(&["work"]);
    let personal = zone(&["personal"]);
    let manager = ProviderRegistryManager::new(registry(&work, 2));

    assert_eq!(
        manager
            .publish(registry(&work, 2), drain_policy())
            .await
            .err(),
        Some(ProviderRuntimeError::InvalidLifecycleTransition)
    );
    assert_eq!(
        manager
            .publish(registry(&personal, 3), drain_policy())
            .await
            .err(),
        Some(ProviderRuntimeError::InvalidLifecycleTransition)
    );
    assert_eq!(manager.current().lifecycle(), RegistryLifecycle::Accepting);
}

fn forward_request(zone_path: &ZonePath, hops: u32) -> ProviderForwardRequest {
    ProviderForwardRequest::new(
        identity(zone_path, "runtime-a"),
        ForwardTarget::named(
            ResourceTypeName::parse("Process").expect("standard type"),
            ResourceName::parse("worker").expect("valid name"),
        ),
        ZoneLabelId::parse("payments").expect("valid label"),
        hops,
    )
    .with_zone_link_connected(true)
}

// A Provider states where it wants to go. It never states that it may relay:
// `ProviderForwardRequest` has no grant field, and the grants argument is
// produced only by the local RBAC engine.
#[test]
fn a_provider_cannot_self_assert_relay() {
    let work = zone(&["work"]);
    let request = forward_request(&work, 4);

    assert_eq!(
        admit_provider_forward(&request, LocalHopGrants::denied()).err(),
        Some(ZoneRouteFailClosedReason::RelayDenied)
    );

    // A Provider that publishes a method literally named `relay` still gets no
    // relay grant: capability publication is not authorization.
    let mut builder = ProviderRegistryBuilder::new(work.clone(), generation(1));
    builder
        .register_instance(descriptor(&work, "runtime-a", 1, &["relay"]), "a")
        .expect("descriptor registers");
    let registry = builder.finish().expect("registry seals");
    registry
        .admit(admission(&work, "runtime-a", "relay"))
        .expect("the provider may invoke its own method named relay");
    assert_eq!(
        admit_provider_forward(&request, LocalHopGrants::denied()).err(),
        Some(ZoneRouteFailClosedReason::RelayDenied)
    );
}

#[test]
fn each_forward_requires_relay_plus_the_target_verb() {
    let work = zone(&["work"]);
    let request = forward_request(&work, 4);

    assert_eq!(
        admit_provider_forward(&request, LocalHopGrants::evaluated(false, true)).err(),
        Some(ZoneRouteFailClosedReason::RelayDenied)
    );
    assert_eq!(
        admit_provider_forward(&request, LocalHopGrants::evaluated(true, false)).err(),
        Some(ZoneRouteFailClosedReason::PolicyDenial)
    );

    let forwarded = admit_provider_forward(&request, LocalHopGrants::evaluated(true, true))
        .expect("both independent grants admit the hop");
    assert_eq!(forwarded.forwarded_remaining_hops(), 3);
    assert_eq!(forwarded.target(), request.target());
    assert_eq!(forwarded.next_hop(), request.next_hop());
}

#[test]
fn every_hop_re_evaluates_both_grants_and_the_budget() {
    let work = zone(&["work"]);
    let mut remaining = 2;
    for _ in 0..2 {
        let request = forward_request(&work, remaining);
        // A previous hop's allow supplies nothing: this hop still needs both.
        assert_eq!(
            admit_provider_forward(&request, LocalHopGrants::evaluated(true, false)).err(),
            Some(ZoneRouteFailClosedReason::PolicyDenial)
        );
        remaining = admit_provider_forward(&request, LocalHopGrants::evaluated(true, true))
            .expect("hop admits")
            .forwarded_remaining_hops();
    }
    assert_eq!(remaining, 0);
    assert_eq!(
        admit_provider_forward(
            &forward_request(&work, remaining),
            LocalHopGrants::evaluated(true, true)
        )
        .err(),
        Some(ZoneRouteFailClosedReason::HopLimitExceeded)
    );
}

#[test]
fn a_disconnected_uplink_and_an_attachment_offer_fail_closed() {
    let work = zone(&["work"]);
    let disconnected = ProviderForwardRequest::new(
        identity(&work, "runtime-a"),
        ForwardTarget::nameless(ResourceTypeName::parse("Process").expect("standard type")),
        ZoneLabelId::parse("payments").expect("valid label"),
        4,
    );
    assert_eq!(
        admit_provider_forward(&disconnected, LocalHopGrants::evaluated(true, true)).err(),
        Some(ZoneRouteFailClosedReason::ZoneLinkDisconnected)
    );
    assert_eq!(
        admit_provider_forward(
            &forward_request(&work, 4).with_attachment_offer(true),
            LocalHopGrants::evaluated(true, true)
        )
        .err(),
        Some(ZoneRouteFailClosedReason::AttachmentNotPermittedOverZoneLink)
    );
}

#[test]
fn redacted_debug_surfaces_leak_no_identity_or_target() {
    let work = zone(&["work"]);
    let identity = identity(&work, "runtime-a");
    assert_eq!(format!("{identity:?}"), "SessionIdentity(<redacted>)");

    let descriptor = descriptor(&work, "runtime-a", 1, &["start"]);
    let rendered = format!("{descriptor:?}");
    assert!(!rendered.contains("runtime-a"));
    assert!(!rendered.contains("work"));

    let request = forward_request(&work, 4);
    let rendered = format!("{request:?}");
    assert!(!rendered.contains("worker"));
    assert!(!rendered.contains("payments"));
}
