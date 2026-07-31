//! The shared audio semantic Service and Binding base contract.
//!
//! This module owns the common base spec, status, and schema contract for the
//! frozen audio pair `audio.d2bus.org.AudioService` and
//! `audio.d2bus.org.AudioBinding`. The field sets below are the top-level
//! provider-neutral base fields stated by the audio Provider dossier's
//! `AudioService.spec`, `AudioService.status`, `AudioBinding.spec`, and
//! `AudioBinding.status` tables, which are the ResourceType base per D089 and
//! D088.
//!
//! PipeWire aliases, node selectors, portal settings, frontend parameters, and
//! every other implementation detail are rejected from these bases and belong
//! only in an implementation's strict `spec.provider` and `status.provider`
//! extensions.
//!
//! Interiors this catalog does not model. `grants` carries `mic`, `speaker`,
//! `speakerLevel`, and `micGain`; `channels` carries the observed counterparts.
//! Those member names and domains are stated by the dossier, but this catalog
//! freezes only the top-level base field set, so an implementation binds the
//! interior from the dossier rather than from a Rust type here.

use std::sync::OnceLock;

use super::{
    super::provider::BindingTargetType, SemanticFamily, SemanticPairContract,
    SemanticPairDeclaration,
};

/// The dot-qualified API ResourceType of the audio owner authority and
/// consumer projection Service.
pub const AUDIO_SERVICE_RESOURCE_TYPE: &str = "audio.d2bus.org.AudioService";

/// The dot-qualified API ResourceType of the audio local consumer intent
/// Binding.
pub const AUDIO_BINDING_RESOURCE_TYPE: &str = "audio.d2bus.org.AudioBinding";

const SERVICE_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "serviceRole",
    "implementationEndpointRefs",
    "operations",
    "authority",
];
const SERVICE_SPEC_REQUIRED: &[&str] = &[
    "providerRef",
    "serviceRole",
    "implementationEndpointRefs",
    "operations",
];
const SERVICE_STATUS_ALLOWED: &[&str] = &[
    "serviceRole",
    "availability",
    "routeState",
    "implementationEndpointRefs",
    "activeConsumerCount",
    "activeMicCaptureCount",
    "pendingMicRequestCount",
    "pendingMicZoneCount",
];

const BINDING_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "serviceRef",
    "grants",
    "guestUsers",
    "suspendOnGuestAbsent",
];
const BINDING_SPEC_REQUIRED: &[&str] = &["providerRef", "serviceRef", "grants"];
const BINDING_STATUS_ALLOWED: &[&str] = &[
    "channels",
    "enforcementPosture",
    "lastSetApplied",
    "observedServiceRef",
    "realizationRefs",
];

/// A projection Service carries only `providerRef`, its observed role, and its
/// local route Endpoints. It never carries the owner authority descriptor and
/// never carries `spec.provider`.
const PROJECTION_SPEC_ALLOWED: &[&str] =
    &["providerRef", "serviceRole", "implementationEndpointRefs"];
const PROJECTION_SPEC_REQUIRED: &[&str] =
    &["providerRef", "serviceRole", "implementationEndpointRefs"];

/// The owner Service references only same-Zone local implementation
/// `Endpoint` resources.
const ALLOWED_BACKING_REF_TYPES: &[&str] = &["Endpoint"];

/// An audio Binding is per-Guest.
const ALLOWED_BINDING_TARGET_REF_TYPES: &[BindingTargetType] = &[BindingTargetType::Guest];

const DECLARATION: SemanticPairDeclaration = SemanticPairDeclaration {
    family: SemanticFamily::Audio,
    service_type_segment: "AudioService",
    binding_type_segment: "AudioBinding",
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

/// Borrow the frozen audio pair contract.
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

    #[test]
    fn the_pair_names_the_exact_frozen_resource_types() {
        let pair = contract();
        assert_eq!(
            pair.service().resource_type().as_str(),
            AUDIO_SERVICE_RESOURCE_TYPE
        );
        assert_eq!(
            pair.binding().resource_type().as_str(),
            AUDIO_BINDING_RESOURCE_TYPE
        );
    }

    /// Canonical minimal base acceptance without `spec.provider`, plus a
    /// strict serde and canonical-schema round trip.
    #[test]
    fn the_canonical_minimal_base_is_accepted_without_a_provider_extension() {
        assert_minimal_base_round_trips(
            contract().service(),
            r#"{"implementationEndpointRefs":["Endpoint/audio-authority"],"operations":["playback"],"serviceRole":"owner"}"#,
        );
        assert_minimal_base_round_trips(
            contract().binding(),
            r#"{"grants":{"mic":"off","speaker":"on"},"serviceRef":"audio.d2bus.org.AudioService/host-audio"}"#,
        );
    }

    /// Each initial and fake alternate Provider passes the identical base
    /// conformance fixture.
    #[test]
    fn every_implementation_passes_the_identical_base_fixture() {
        assert_base_is_provider_neutral(
            contract(),
            r#"{"implementationEndpointRefs":["Endpoint/audio-authority"],"operations":["playback"],"serviceRole":"owner"}"#,
            r#"{"grants":{"mic":"off","speaker":"on"},"serviceRef":"audio.d2bus.org.AudioService/host-audio"}"#,
            "audio-pipewire",
            "audio-alternate",
        );
    }

    /// Implementation-detail rejection in the base spec.
    #[test]
    fn a_pipewire_detail_is_not_a_base_field() {
        let contract = contract()
            .service()
            .schema_contract(std::iter::empty())
            .unwrap();
        let spec = crate::v3::resource::ResourceSpec::new(
            Some(provider_ref("audio-pipewire")),
            None,
            object(
                r#"{"captureAlias":"default","implementationEndpointRefs":["Endpoint/a"],"operations":["playback"],"serviceRole":"owner"}"#,
            ),
            None,
        )
        .unwrap();
        assert!(contract.validate_minimal_base_spec(&spec).is_err());
    }

    /// Common fields only under `status.resource`; implementation observation
    /// only under `status.provider`.
    #[test]
    fn a_pipewire_observation_is_not_a_common_status_field() {
        let status = contract().service().status();
        assert!(
            status
                .validate_names(["availability", "serviceRole"])
                .is_ok()
        );
        assert_eq!(
            status.validate_names(["pipeWireSession"]),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// Owner versus projection discrimination: a projection may not carry the
    /// owner authority descriptor.
    #[test]
    fn a_projection_may_not_carry_the_owner_authority_descriptor() {
        let spec = crate::v3::resource::ResourceSpec::new(
            Some(provider_ref("audio-pipewire")),
            None,
            object(
                r#"{"authority":{},"implementationEndpointRefs":["Endpoint/a"],"serviceRole":"projection"}"#,
            ),
            None,
        )
        .unwrap();
        assert_eq!(
            contract().projection().validate_projection_spec(&spec),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// The projection factory type binding is derivable for this family.
    #[test]
    fn the_projection_factory_binds_the_frozen_types() {
        let factory = contract().projection().projection_factory().unwrap();
        assert_eq!(factory.service_type().as_str(), AUDIO_SERVICE_RESOURCE_TYPE);
        assert_eq!(factory.binding_type().as_str(), AUDIO_BINDING_RESOURCE_TYPE);
        assert_eq!(factory.allowed_backing_ref_types().len(), 1);
    }
}
