//! The provider-owned implementation of the Endpoint family's driver
//! effects (U6): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Three surfaces share one implementation value:
//!
//! - the driver's typed seam, [`EndpointDriverEffects`] and
//!   [`EndpointPurposeVocabulary`], which the family's driver holds (the
//!   factory builds it from the same facets);
//! - the purpose derivations the family classifies against
//!   ([`guest_control_producer`], [`device_worker_endpoint_class`]): the
//!   closed admission set is derived from the declaring providers' own
//!   vocabularies - the Cloud Hypervisor provider's child roles and the
//!   Device TPM Provider's declared worker-socket purposes - so it cannot
//!   drift from the children those providers commit;
//! - the declared zone-plane service [`ENDPOINT_EFFECTS_SERVICE`], hosted
//!   per zone by the daemon through [`EndpointEffectsServiceFactory`]. Its
//!   one method (`inspect-endpoint`) answers the family's committed purpose
//!   vocabulary and realization inventory - the same derivations the driver
//!   effects classify against, served from inside the owning crate.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]): the host socket surface and the two
//! row-evidence probes the daemon supplies. Nothing here names a daemon
//! state type.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_guest_cloud_hypervisor::ChildRole;
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::{EndpointDriverEffects, EndpointPurposeVocabulary, GuestControlProducer};
use crate::endpoint::EndpointClass;
use crate::facets::EndpointEffectFacets;

/// The bounded budget one endpoint realization waits for its evidence
/// before reporting a retryable failure (the actor owns the retry, R13).
const SOCKET_REALIZE_BUDGET: Duration = Duration::from_secs(5);

/// The producer the Cloud Hypervisor provider's own child-role vocabulary
/// declares for one guest-runtime control purpose, or `None` for any other
/// purpose. Derived from the provider's role list, so the closed family
/// cannot drift from the children a guest's provider controller commits.
pub fn guest_control_producer(purpose: &str) -> Option<GuestControlProducer> {
    for (role, producer) in [
        (ChildRole::ChApiEndpoint, GuestControlProducer::VmmProcess),
        (ChildRole::GuestControlEndpoint, GuestControlProducer::Guest),
    ] {
        if role.purpose() == Some(purpose) {
            return Some(producer);
        }
    }
    None
}

/// Whether one purpose belongs to the guest-runtime control family.
pub fn guest_control_purpose(purpose: &str) -> bool {
    guest_control_producer(purpose).is_some()
}

/// The Endpoint class the Device TPM Provider declares for one of its
/// device-worker purposes, or `None` for any other purpose.
///
/// Derived from the provider's own constants (`TPM_SERVER_ENDPOINT_PURPOSE`,
/// `TPM_CONTROL_ENDPOINT_PURPOSE`), so the closed realization set cannot
/// drift from the rows the provider declares in its projection.
pub fn device_worker_endpoint_class(purpose: &str) -> Option<EndpointClass> {
    match purpose {
        d2b_provider_device_tpm::TPM_SERVER_ENDPOINT_PURPOSE => Some(EndpointClass::Device),
        d2b_provider_device_tpm::TPM_CONTROL_ENDPOINT_PURPOSE => Some(EndpointClass::Control),
        _ => None,
    }
}

/// Whether one purpose belongs to the device-worker family.
pub fn device_worker_purpose(purpose: &str) -> bool {
    device_worker_endpoint_class(purpose).is_some()
}

/// The Endpoint family's declared effects service.
///
/// One zone-plane method, `inspect-endpoint`: it answers this family's
/// committed purpose vocabulary - the purposes the declaring providers
/// commit and the producer/class each derives to - plus the closed
/// realization inventory the driver admits. The report is served from the
/// crate's own derivations, so it proves the purpose vocabulary runs inside
/// the owning crate (U6), and it is hermetic: no host state is read or
/// mutated.
///
/// The service is declared on the `Endpoint` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const ENDPOINT_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "endpoint.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-endpoint")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-endpoint` response payload: the family's committed
/// purpose vocabulary and realization inventory. The payload is built
/// through the canonical JSON object path, so a structural character in a
/// committed purpose yields a correctly escaped report rather than an
/// unparseable one; the refusal is unreachable and names its own code.
fn inspect_endpoint_response() -> Result<EffectResponse, EffectServiceError> {
    let purpose_entry = |class: &str, producer: Option<(&str, &str)>| {
        let mut entry = serde_json::Map::new();
        entry.insert("class".to_owned(), serde_json::Value::String(class.to_owned()));
        if let Some((producer, locality)) = producer {
            entry.insert(
                "producer".to_owned(),
                serde_json::Value::String(producer.to_owned()),
            );
            entry.insert(
                "locality".to_owned(),
                serde_json::Value::String(locality.to_owned()),
            );
        }
        entry
    };
    let payload = serde_json::from_value(serde_json::json!({
        "family": "endpoint",
        "resourceType": "Endpoint",
        "purposes": {
            "ch-api": purpose_entry("control", Some(("Process", "host-local"))),
            "guest-control": purpose_entry("control", Some(("Guest", "cross-domain"))),
            "swtpm-tpm-socket": purpose_entry("device", None),
            "swtpm-control-socket": purpose_entry("control", None),
        },
        "realizations": ["virtiofsd-socket", "guest-control", "device-worker-socket"],
    }))
    .map_err(|_| EffectServiceError::Declined {
        service: ENDPOINT_EFFECTS_SERVICE.id.to_owned(),
        reason: "inspect-endpoint-response-invalid".to_owned(),
    })?;
    Ok(EffectResponse::new(payload))
}

/// The provider-owned Endpoint effects (U6), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`EndpointEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same realization.
pub struct EndpointEffectsService {
    socket: Arc<dyn crate::facets::EndpointSocketSource>,
    guest_vmm: Arc<dyn crate::facets::GuestVmmEvidenceSource>,
    device_worker: Arc<dyn crate::facets::DeviceWorkerEvidenceSource>,
}

impl EndpointEffectsService {
    /// Build the effects from one zone's daemon-supplied facet set (R2):
    /// every daemon-structural read rides the facets, never a daemon handle.
    pub fn new(facets: EndpointEffectFacets) -> Self {
        Self {
            socket: facets.socket,
            guest_vmm: facets.guest_vmm,
            device_worker: facets.device_worker,
        }
    }

    /// Whether one purpose's evidence row reports `Ready` at its current
    /// generation, through the facet that owns the evidence.
    async fn evidence_present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        if guest_control_purpose(purpose) {
            self.guest_vmm.present(producer_ref, purpose).await
        } else if device_worker_purpose(purpose) {
            self.device_worker.present(producer_ref, purpose).await
        } else {
            false
        }
    }
}

impl EndpointPurposeVocabulary for EndpointEffectsService {
    fn guest_control_producer(&self, purpose: &str) -> Option<GuestControlProducer> {
        guest_control_producer(purpose)
    }

    fn device_worker_endpoint_class(&self, purpose: &str) -> Option<EndpointClass> {
        device_worker_endpoint_class(purpose)
    }
}

#[async_trait]
impl EndpointDriverEffects for EndpointEffectsService {
    async fn socket_present(&self, producer_ref: &ResourceRef, purpose: &str) -> bool {
        if guest_control_purpose(purpose) || device_worker_purpose(purpose) {
            return self.evidence_present(producer_ref, purpose).await;
        }
        self.socket.present(producer_ref, purpose).await
    }

    async fn ensure_socket(
        &self,
        producer_ref: &ResourceRef,
        purpose: &str,
    ) -> Result<(), String> {
        if guest_control_purpose(purpose) || device_worker_purpose(purpose) {
            // The evidence family realizes through its producer row: the
            // nested VMM or the worker launch is live exactly while the
            // committed row reports Ready, so ensure waits the bounded
            // budget for that evidence and reports a retryable failure
            // otherwise (R13).
            let deadline = tokio::time::Instant::now() + SOCKET_REALIZE_BUDGET;
            loop {
                if self.evidence_present(producer_ref, purpose).await {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "endpoint {purpose:?} is not realized within its realize budget"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        self.socket.ensure(producer_ref, purpose).await
    }

    async fn remove_socket(
        &self,
        producer_ref: &ResourceRef,
        purpose: &str,
    ) -> Result<(), String> {
        if guest_control_purpose(purpose) || device_worker_purpose(purpose) {
            // The evidence families are owned by the producer (the guest's
            // nested VMM or the worker launch); the daemon creates nothing
            // to remove, so their removal converges without effects.
            return Ok(());
        }
        self.socket.remove(producer_ref, purpose).await
    }
}

#[async_trait]
impl EffectService for EndpointEffectsService {
    async fn handle(
        &self,
        _invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // crate's own purpose derivations.
        inspect_endpoint_response()
    }
}

/// The composition-root factory that hosts the Endpoint effects service in
/// one zone (R5): the daemon registers one per zone, carrying that zone's
/// facet set, and the host rebuilds the service from it on respawn.
pub struct EndpointEffectsServiceFactory {
    facets: EndpointEffectFacets,
}

impl EndpointEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: EndpointEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for EndpointEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(EndpointEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::FakeSocketEffects;

    /// The derived families are exactly the declaring providers' own
    /// vocabularies: the Cloud Hypervisor child-role purposes and the Device
    /// TPM Provider's declared worker-socket purposes.
    #[test]
    fn the_derived_families_follow_the_declaring_providers() {
        assert_eq!(
            guest_control_producer("ch-api"),
            Some(GuestControlProducer::VmmProcess)
        );
        assert_eq!(
            guest_control_producer("guest-control"),
            Some(GuestControlProducer::Guest)
        );
        assert_eq!(
            GuestControlProducer::VmmProcess.resource_type(),
            ChildRole::VmmProcess.resource_type(),
            "the family's VMM producer is the Process the provider commits"
        );
        assert!(guest_control_purpose("ch-api"));
        assert!(guest_control_purpose("guest-control"));
        assert!(!guest_control_purpose("virtiofsd"));
        assert!(!guest_control_purpose("aca-sandbox-agent"));

        assert_eq!(
            device_worker_endpoint_class(d2b_provider_device_tpm::TPM_SERVER_ENDPOINT_PURPOSE),
            Some(EndpointClass::Device)
        );
        assert_eq!(
            device_worker_endpoint_class(d2b_provider_device_tpm::TPM_CONTROL_ENDPOINT_PURPOSE),
            Some(EndpointClass::Control)
        );
        assert!(device_worker_purpose("swtpm-tpm-socket"));
        assert!(device_worker_purpose("swtpm-control-socket"));
        assert!(!device_worker_purpose("virtiofsd"));
        assert!(!device_worker_purpose("ch-api"));
    }

    /// The string value of one canonical JSON field, if it is a string.
    fn json_string(
        value: Option<&d2b_contracts_resource::v3::CanonicalJsonValue>,
    ) -> Option<&str> {
        match value {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// The `inspect-endpoint` payload rows equal the crate's own
    /// derivations: every purpose the report commits derives to the same
    /// class, producer, and locality the driver effects classify against,
    /// and the committed purpose set is exactly the union of the two
    /// derived families. A provider role or purpose rename that drifts the
    /// report fails this pin.
    #[test]
    fn the_inspect_payload_rows_match_the_purpose_derivations() {
        let response = inspect_endpoint_response().expect("the payload is canonical JSON");
        let purposes = response
            .payload
            .get("purposes")
            .and_then(|value| value.as_object())
            .expect("purposes map");

        let mut family_purposes = std::collections::BTreeSet::new();
        for role in [ChildRole::ChApiEndpoint, ChildRole::GuestControlEndpoint] {
            family_purposes.insert(role.purpose().expect("declared purpose"));
        }
        family_purposes.insert(d2b_provider_device_tpm::TPM_SERVER_ENDPOINT_PURPOSE);
        family_purposes.insert(d2b_provider_device_tpm::TPM_CONTROL_ENDPOINT_PURPOSE);
        assert_eq!(
            purposes
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            family_purposes,
            "the committed purpose set is exactly the union of the derived families"
        );

        for (purpose, row) in purposes {
            let row = row.as_object().expect("purpose row");
            let class = json_string(row.get("class"));
            if let Some(producer) = guest_control_producer(purpose) {
                assert_eq!(class, Some("control"), "{purpose} is a guest-control purpose");
                assert_eq!(
                    json_string(row.get("producer")),
                    Some(producer.resource_type()),
                    "{purpose} producer derives from the provider role"
                );
                assert_eq!(
                    json_string(row.get("locality")),
                    serde_json::to_value(producer.locality())
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .as_deref(),
                    "{purpose} locality derives from the producer"
                );
            } else {
                let derived = device_worker_endpoint_class(purpose)
                    .expect("every committed purpose belongs to a derived family");
                assert_eq!(
                    class,
                    serde_json::to_value(derived)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .as_deref(),
                    "{purpose} class derives from the device-worker vocabulary"
                );
                assert!(row.get("producer").is_none(), "{purpose} has no producer");
            }
        }
    }

    /// The service's vocabulary answers equal the free derivations, and the
    /// socket dispatch routes the evidence purposes onto the evidence
    /// facets and everything else onto the host socket facet.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_service_dispatches_onto_the_facets_by_purpose() {
        let fake = FakeSocketEffects::new();
        let service = EndpointEffectsService::new(fake.facet_set());
        let producer = ResourceRef::parse("Process/acceptance-guest-vmm").expect("producer");

        assert_eq!(
            service.guest_control_producer("ch-api"),
            Some(GuestControlProducer::VmmProcess)
        );
        assert_eq!(
            service.device_worker_endpoint_class(
                d2b_provider_device_tpm::TPM_SERVER_ENDPOINT_PURPOSE
            ),
            Some(EndpointClass::Device)
        );
        // The evidence purposes never reach the socket facet: the scripted
        // socket double records no call for them.
        assert!(
            !service.socket_present(&producer, "ch-api").await,
            "the guest-control evidence answers through its own facet"
        );
        assert_eq!(fake.call_order(), Vec::<&'static str>::new());
        // The virtiofsd purpose reaches the host socket facet.
        assert!(!service.socket_present(&producer, "virtiofsd").await);
        assert_eq!(fake.call_order(), ["socket-present"]);
    }
}