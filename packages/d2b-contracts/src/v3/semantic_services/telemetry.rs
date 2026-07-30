//! The shared telemetry semantic Service and Binding base contract.
//!
//! This module owns the common base spec, status, and schema contract for the
//! frozen telemetry pair `telemetry.d2bus.org.TelemetryService` and
//! `telemetry.d2bus.org.TelemetryBinding`. The spec field sets below are the
//! "TelemetryService base spec" and "TelemetryBinding base spec" D089 tables
//! of the telemetry Provider dossier.
//!
//! OTEL, OTLP, and backend-product choices are not base fields. They belong
//! only in an implementation's strict `spec.provider` extension.
//!
//! Interiors this catalog does not model. `signals` is the non-empty subset of
//! metrics, traces, and logs. `quota` and `policy` are named as required base
//! objects, but the dossier states their contents as prose rather than as a
//! frozen member table, so this catalog freezes the top-level field only.
//!
//! Status field names this catalog could not determine. The dossier describes
//! `TelemetryService.status.resource` and `TelemetryBinding.status.resource`
//! in prose. Only `serviceRole` and `serviceReadiness` are stated as field
//! spellings; the effective signal, quota, and policy digests, the ingest and
//! import readiness summaries, the producer counts, the queue and drop
//! counters, and the Binding's observed generations, occupancy, and stamping
//! flag are described without frozen names. The common status layers below
//! therefore carry only the stated spellings and reject the rest, which is the
//! fail-closed posture; the missing names are a specification gap, not an
//! invitation to choose them here.

use std::sync::OnceLock;

use super::{
    super::provider::BindingTargetType, SemanticFamily, SemanticPairContract,
    SemanticPairDeclaration,
};

/// The dot-qualified API ResourceType of the telemetry ingest authority and
/// consumer projection Service.
pub const TELEMETRY_SERVICE_RESOURCE_TYPE: &str = "telemetry.d2bus.org.TelemetryService";

/// The dot-qualified API ResourceType of the telemetry local producer intent
/// Binding.
pub const TELEMETRY_BINDING_RESOURCE_TYPE: &str = "telemetry.d2bus.org.TelemetryBinding";

const SERVICE_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "serviceRole",
    "ingestEndpointRefs",
    "signals",
    "quota",
    "policy",
    "authorityDescriptor",
];
const SERVICE_SPEC_REQUIRED: &[&str] =
    &["providerRef", "serviceRole", "signals", "quota", "policy"];
const SERVICE_STATUS_ALLOWED: &[&str] = &["serviceRole", "serviceReadiness"];

const BINDING_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "serviceRef",
    "producerRef",
    "signals",
    "quota",
    "policy",
];
const BINDING_SPEC_REQUIRED: &[&str] = &[
    "providerRef",
    "serviceRef",
    "producerRef",
    "signals",
    "quota",
    "policy",
];
const BINDING_STATUS_ALLOWED: &[&str] = &[];

/// A projection Service has no ingest Endpoints and no authority descriptor.
/// Core derives its signal, quota, and policy ceiling from the admitted
/// export.
const PROJECTION_SPEC_ALLOWED: &[&str] =
    &["providerRef", "serviceRole", "signals", "quota", "policy"];
const PROJECTION_SPEC_REQUIRED: &[&str] =
    &["providerRef", "serviceRole", "signals", "quota", "policy"];

/// The authority Service references same-Zone local telemetry-ingest
/// `Endpoint` resources.
const ALLOWED_BACKING_REF_TYPES: &[&str] = &["Endpoint"];

/// A telemetry Binding's producer is a same-Zone Zone or Guest.
const ALLOWED_BINDING_TARGET_REF_TYPES: &[BindingTargetType] =
    &[BindingTargetType::Guest, BindingTargetType::Zone];

const DECLARATION: SemanticPairDeclaration = SemanticPairDeclaration {
    family: SemanticFamily::Telemetry,
    service_type_segment: "TelemetryService",
    binding_type_segment: "TelemetryBinding",
    service_spec_allowed: SERVICE_SPEC_ALLOWED,
    service_spec_required: SERVICE_SPEC_REQUIRED,
    service_status_allowed: SERVICE_STATUS_ALLOWED,
    binding_spec_allowed: BINDING_SPEC_ALLOWED,
    binding_spec_required: BINDING_SPEC_REQUIRED,
    binding_status_allowed: BINDING_STATUS_ALLOWED,
    allowed_backing_ref_types: Some(ALLOWED_BACKING_REF_TYPES),
    allowed_binding_target_ref_types: ALLOWED_BINDING_TARGET_REF_TYPES,
    projection_spec_allowed: PROJECTION_SPEC_ALLOWED,
    projection_spec_required: PROJECTION_SPEC_REQUIRED,
};

/// Borrow the frozen telemetry pair contract.
pub fn contract() -> &'static SemanticPairContract {
    static CONTRACT: OnceLock<SemanticPairContract> = OnceLock::new();
    CONTRACT.get_or_init(|| SemanticPairContract::build(&DECLARATION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::semantic_services::SemanticContractError;
    use crate::v3::semantic_services::tests_support::{
        assert_base_is_provider_neutral, assert_minimal_base_round_trips, object, provider_ref,
    };

    const MINIMAL_SERVICE: &str =
        r#"{"policy":{},"quota":{},"serviceRole":"authority","signals":["metrics"]}"#;
    const MINIMAL_BINDING: &str = r#"{"policy":{},"producerRef":"Zone/work","quota":{},"serviceRef":"telemetry.d2bus.org.TelemetryService/ingest","signals":["metrics"]}"#;

    #[test]
    fn the_pair_names_the_exact_frozen_resource_types() {
        let pair = contract();
        assert_eq!(
            pair.service().resource_type().as_str(),
            TELEMETRY_SERVICE_RESOURCE_TYPE
        );
        assert_eq!(
            pair.binding().resource_type().as_str(),
            TELEMETRY_BINDING_RESOURCE_TYPE
        );
    }

    /// Canonical minimal base acceptance without `spec.provider`.
    #[test]
    fn the_canonical_minimal_base_is_accepted_without_a_provider_extension() {
        assert_minimal_base_round_trips(contract().service(), MINIMAL_SERVICE);
        assert_minimal_base_round_trips(contract().binding(), MINIMAL_BINDING);
    }

    /// Each initial and fake alternate Provider passes the identical base
    /// conformance fixture.
    #[test]
    fn every_implementation_passes_the_identical_base_fixture() {
        assert_base_is_provider_neutral(
            contract(),
            MINIMAL_SERVICE,
            MINIMAL_BINDING,
            "observability-otel",
            "telemetry-alternate",
        );
    }

    /// Implementation-detail rejection: the backend product and wire protocol
    /// are not base fields.
    #[test]
    fn a_backend_or_wire_protocol_is_not_a_base_field() {
        for detail in ["backend", "ingestProtocol", "backendEndpointRefs"] {
            assert_eq!(
                contract().service().spec().validate_names([
                    "providerRef",
                    "serviceRole",
                    "signals",
                    "quota",
                    "policy",
                    detail,
                ]),
                Err(SemanticContractError::SchemaViolation)
            );
        }
    }

    /// Owner versus projection discrimination: a projection carries no local
    /// ingest Endpoints and no authority descriptor.
    #[test]
    fn a_projection_rejects_ingest_endpoints_and_the_authority_descriptor() {
        for extra in [
            r#""ingestEndpointRefs":["Endpoint/ingest"]"#,
            r#""authorityDescriptor":{}"#,
        ] {
            let base = format!(
                r#"{{"policy":{{}},"quota":{{}},"serviceRole":"projection","signals":["metrics"],{extra}}}"#
            );
            let spec = crate::v3::resource::ResourceSpec::new(
                Some(provider_ref("observability-otel")),
                None,
                object(&base),
                None,
            )
            .unwrap();
            assert_eq!(
                contract().projection().validate_projection_spec(&spec),
                Err(SemanticContractError::SchemaViolation)
            );
        }
    }

    /// The Binding common status layer is closed while the dossier does not
    /// freeze its field spellings.
    #[test]
    fn the_binding_common_status_layer_is_closed_pending_frozen_names() {
        assert!(contract().binding().status().validate_names([]).is_ok());
        assert_eq!(
            contract().binding().status().validate_names(["stamped"]),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// The projection factory type binding is derivable for this family.
    #[test]
    fn the_projection_factory_binds_the_frozen_types() {
        let factory = contract().projection().projection_factory().unwrap();
        assert_eq!(
            factory.service_type().as_str(),
            TELEMETRY_SERVICE_RESOURCE_TYPE
        );
        assert_eq!(
            factory.binding_type().as_str(),
            TELEMETRY_BINDING_RESOURCE_TYPE
        );
        assert_eq!(factory.allowed_binding_target_ref_types().len(), 2);
    }
}
