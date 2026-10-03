//! Canonical admitted-binding fixtures for the telemetry delivery route.
//!
//! A [`DeliveryRoute`](crate::DeliveryRoute) has no constructor reachable
//! from desired fields, so every test that needs a live route has to build
//! one the way production does: run [`admit_binding_request`] with a real
//! authorization grant, a source admission scoped to the exact
//! [`BindingKey`], the realization's declared support, and the freshness
//! evidence the relationship is fenced against.
//!
//! Keeping that construction in one place is what makes the tests meaningful:
//! a route that admits here is admitted by the shared contract, not by a
//! helper that could drift from it.

use d2b_contracts_resource::v3::{
    BindingArbitration, BindingAuthorization, BindingContractError, BindingEvidence, BindingKey,
    BindingKind, BindingLifecycleState, BindingObservation, BindingRealizationFacet,
    BindingRealizationSupport, BindingRefusal, BindingSlot, BindingSpecFingerprint, CompletionCondition,
    DesiredDigest, DesiredRevision, FreshnessTuple, ReleaseOutcome, RequestedRights,
    ResourceRef, ResourceUid, SourceAdmission, SourceReservation, StoreIncarnation, ZoneId,
    admit_binding_request,
};

use crate::DeliverySource;

/// The Zone every fixture relationship belongs to.
pub fn zone() -> ZoneId {
    ZoneId::parse("dev").expect("canonical Zone")
}

/// The source's store identity for one fixture relationship.
pub fn source_uid() -> ResourceUid {
    ResourceUid::from_bytes(&[0x11; 16]).expect("canonical fixture source uid")
}

/// The consumer's store identity for one fixture relationship.
pub fn consumer_uid() -> ResourceUid {
    ResourceUid::from_bytes(&[0x22; 16]).expect("canonical fixture consumer uid")
}

/// A store generation every fixture relationship is fenced against.
pub fn store_incarnation() -> StoreIncarnation {
    StoreIncarnation::parse("fixture-store").expect("bounded store incarnation")
}

/// The committed-row freshness tuple for one fixture relationship.
///
/// The digest is a non-secret content commitment, so this names which bytes
/// were committed without naming the row's content.
pub fn freshness(resource: &ResourceRef) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        store_incarnation(),
        resource.clone(),
        source_uid(),
        DesiredRevision::INITIAL,
        DesiredDigest::of(&canonical_bytes(&serde_json::json!({
            "resourceRef": resource.to_canonical_string(),
        }))),
    )
}

fn canonical_bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a typed JSON value always serializes")
}

/// The realization support a source provider declares for its own bindings.
///
/// An empty set is the "declares nothing" case: a request needing any facet
/// is refused at `Prepare`, which is how an unsupported provider facet is
/// caught rather than approximated.
pub fn support(facets: &[BindingRealizationFacet]) -> BindingRealizationSupport {
    BindingRealizationSupport::new(facets.to_vec())
        .expect("the fixture facet set is duplicate-free")
}

/// The one `BindingKey` for one fixture relationship.
///
/// # Panics
///
/// Panics when the fixture pair is not an admitted source/consumer pairing
/// for the kind, which would be a fixture bug rather than a behavior under
/// test.
pub fn binding_key(kind: BindingKind, source: &ResourceRef, consumer: &ResourceRef) -> BindingKey {
    BindingKey::new(
        zone(),
        kind,
        source.clone(),
        source_uid(),
        consumer.clone(),
        consumer_uid(),
        slot(kind),
    )
    .expect("the fixture pair is an admitted consumer for its kind")
}

fn slot(kind: BindingKind) -> BindingSlot {
    let name = match kind {
        BindingKind::Volume => "config",
        BindingKind::Device => "device",
        BindingKind::Network => "network",
        BindingKind::Endpoint => "ingest",
        BindingKind::Credential => "exporter",
    };
    BindingSlot::parse(name).expect("bounded fixture slot")
}

/// Admit one relationship the way a source provider would.
///
/// # Panics
///
/// Panics when the fixture inputs do not produce an admission. A fixture that
/// cannot be admitted is a fixture bug; the tests that exercise refusal
/// construct the refusal directly rather than through this helper.
pub fn admit(
    key: &BindingKey,
    rights: RequestedRights,
    facets: &[BindingRealizationFacet],
    authorization: &BindingAuthorization,
) -> BindingEvidence {
    let source = SourceAdmission::new(key.clone(), vec![rights], BindingArbitration::Shared)
        .expect("the fixture admits exactly one right");
    let dependencies = vec![freshness(key.source_ref())];
    let admission = admit_binding_request(
        key,
        rights,
        facets,
        authorization,
        &source,
        &support(facets),
        &dependencies,
    )
    .expect("the fixture relationship is admissible");
    let reservation = SourceReservation::new(zone(), key.source_uid().clone(), reservation_token());
    BindingEvidence::admitted(admission, reservation)
}

fn reservation_token() -> d2b_contracts_resource::v3::BoundedToken {
    d2b_contracts_resource::v3::BoundedToken::parse("fixture-reservation")
        .expect("bounded fixture reservation token")
}

/// Advance one admitted relationship to an observed lifecycle state.
///
/// The observation is what a source publishes after its own effects; the
/// resource-owner rows a driver reads are not a substitute for it.
pub fn observe(evidence: &BindingEvidence, state: BindingLifecycleState) -> BindingEvidence {
    let complete = if state.admits_new_use() {
        CompletionCondition::Complete
    } else {
        CompletionCondition::Pending
    };
    let release = if matches!(state, BindingLifecycleState::Released) {
        ReleaseOutcome::Released
    } else {
        ReleaseOutcome::Outstanding
    };
    evidence.clone().observed(BindingObservation::new(
        state,
        complete,
        complete,
        release,
    ))
}

/// The exact endpoint binding a telemetry route delivers over.
pub fn admitted_endpoint_evidence() -> BindingEvidence {
    let source = ResourceRef::parse("Endpoint/ingest").expect("canonical Endpoint");
    let consumer = ResourceRef::parse("Process/collector").expect("canonical consumer");
    let key = binding_key(BindingKind::Endpoint, &source, &consumer);
    admit(
        &key,
        RequestedRights::Consume,
        &[BindingRealizationFacet::EndpointDescriptor],
        &BindingAuthorization::granted(),
    )
}

/// The exact network binding a telemetry route's egress rides.
pub fn admitted_network_evidence() -> BindingEvidence {
    let source = ResourceRef::parse("Network/zone").expect("canonical Network");
    let consumer = ResourceRef::parse("Process/forwarder").expect("canonical consumer");
    let key = binding_key(BindingKind::Network, &source, &consumer);
    admit(
        &key,
        RequestedRights::Consume,
        &[BindingRealizationFacet::SharedFabric],
        &BindingAuthorization::granted(),
    )
}

/// The exact credential binding a telemetry exporter authenticates with.
pub fn admitted_credential_evidence() -> BindingEvidence {
    let source = ResourceRef::parse("Credential/otlp").expect("canonical Credential");
    let consumer = ResourceRef::parse("Process/forwarder").expect("canonical consumer");
    let key = binding_key(BindingKind::Credential, &source, &consumer);
    admit(
        &key,
        RequestedRights::Consume,
        &[BindingRealizationFacet::CredentialDelivery],
        &BindingAuthorization::granted(),
    )
}

/// The freshness evidence one admitted fixture relationship is current
/// against.
///
/// A relationship missing from this list is one whose committed revision
/// moved, which is exactly the dependency change that must stop delivery.
pub fn observed_for(evidence: &BindingEvidence) -> Vec<FreshnessTuple> {
    vec![freshness(evidence.key().source_ref())]
}

/// The freshness evidence every relationship in one set is current against.
///
/// A route is fenced against each of its admitted relationships, so a
/// complete observed set names all of them.
pub fn observed_for_all(evidence: &[&BindingEvidence]) -> Vec<FreshnessTuple> {
    evidence
        .iter()
        .map(|evidence| freshness(evidence.key().source_ref()))
        .collect()
}

/// A relationship refused by the shared contract rather than by a fixture.
///
/// This is the shape a real refusal has: a stage and a field-free reason.
pub fn contract_refusal(
    stage: d2b_contracts_resource::v3::AdmissionStage,
    reason: d2b_contracts_resource::v3::RefusalReason,
) -> BindingRefusal {
    BindingRefusal::new(stage, reason)
}

/// The contract error a malformed fixture request produces.
pub fn contract_error() -> BindingContractError {
    BindingContractError::WrongResourceType
}

/// The declaration digest a fixture request is fingerprinted with.
pub fn fingerprint<T: serde::Serialize>(request: &T) -> BindingSpecFingerprint {
    BindingSpecFingerprint::from_request(request)
}

/// The delivery source a fixture relationship belongs to.
pub fn source_of(kind: BindingKind) -> DeliverySource {
    match kind {
        BindingKind::Endpoint => DeliverySource::Endpoint,
        BindingKind::Network => DeliverySource::Network,
        BindingKind::Credential => DeliverySource::Credential,
        BindingKind::Volume | BindingKind::Device => {
            panic!("{kind:?} is not a telemetry delivery source")
        }
    }
}
