//! The shared security-key semantic Service and Binding base contract.
//!
//! This module owns the common base spec, status, and schema contract for the
//! frozen security-key pair `security-key.d2bus.org.SecurityKeyService` and
//! `security-key.d2bus.org.SecurityKeyBinding`. The field sets below are the
//! top-level provider-neutral base fields stated by the security-key Provider
//! dossier's Service and Binding spec/status contract sections.
//!
//! The Service base is a discriminated `mode` union declaring only semantic
//! security-key authority. The physical backing selector is deliberately not a
//! base field for this family: the dossier places `deviceRef` and the relay
//! Endpoint inside the implementation's strict `spec.provider` extension.
//!
//! Consequences of that placement. Because no semantic base field names a
//! backing resource, this family's closed `allowedBackingRefTypes` set is
//! determinate and empty.
//!
//! Interiors this catalog does not model. `authority` is the D097 descriptor,
//! `target` carries the consuming Guest and User references, and `policy`
//! carries the attachment policy. Their member names appear in the dossier's
//! examples but are not frozen as a base field-name table, so this catalog
//! freezes only the top-level base field set.

use std::sync::OnceLock;

use super::{
    super::provider::BindingTargetType, SemanticBackingDeclaration, SemanticFamily,
    SemanticPairContract, SemanticPairDeclaration,
};

/// The dot-qualified API ResourceType of the security-key owner authority and
/// consumer projection Service.
pub const SECURITY_KEY_SERVICE_RESOURCE_TYPE: &str = "security-key.d2bus.org.SecurityKeyService";

/// The dot-qualified API ResourceType of the security-key local consumer
/// intent Binding.
pub const SECURITY_KEY_BINDING_RESOURCE_TYPE: &str = "security-key.d2bus.org.SecurityKeyBinding";

const SERVICE_SPEC_ALLOWED: &[&str] = &["providerRef", "updatePolicy", "mode", "authority"];
const SERVICE_SPEC_REQUIRED: &[&str] = &["providerRef", "mode"];
const SERVICE_STATUS_ALLOWED: &[&str] = &["authority", "import"];

const BINDING_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "serviceRef",
    "target",
    "policy",
];
const BINDING_SPEC_REQUIRED: &[&str] = &["providerRef", "serviceRef", "target"];
const BINDING_STATUS_ALLOWED: &[&str] = &["attachment"];

/// The Core-owned projection branch permits only `providerRef` and the
/// observed mode. It rejects `spec.provider`, the physical device selector,
/// the authority descriptor, and every physical selector.
const PROJECTION_SPEC_ALLOWED: &[&str] = &["providerRef", "mode"];
const PROJECTION_SPEC_REQUIRED: &[&str] = &["providerRef", "mode"];

/// A security-key Binding targets a consuming Guest and User.
const ALLOWED_BINDING_TARGET_REF_TYPES: &[BindingTargetType] =
    &[BindingTargetType::Guest, BindingTargetType::User];

const DECLARATION: SemanticPairDeclaration = SemanticPairDeclaration {
    family: SemanticFamily::SecurityKey,
    service_type_segment: "SecurityKeyService",
    binding_type_segment: "SecurityKeyBinding",
    service_spec_allowed: SERVICE_SPEC_ALLOWED,
    service_spec_required: SERVICE_SPEC_REQUIRED,
    service_status_allowed: SERVICE_STATUS_ALLOWED,
    binding_spec_allowed: BINDING_SPEC_ALLOWED,
    binding_spec_required: BINDING_SPEC_REQUIRED,
    binding_status_allowed: BINDING_STATUS_ALLOWED,
    backing: SemanticBackingDeclaration::NoBacking,
    allowed_binding_target_ref_types: ALLOWED_BINDING_TARGET_REF_TYPES,
    projection_spec_allowed: PROJECTION_SPEC_ALLOWED,
    projection_spec_required: PROJECTION_SPEC_REQUIRED,
};

/// Borrow the frozen security-key pair contract.
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
        resource_envelope,
    };

    #[test]
    fn the_pair_names_the_exact_frozen_resource_types() {
        let pair = contract();
        assert_eq!(
            pair.service().resource_type().as_str(),
            SECURITY_KEY_SERVICE_RESOURCE_TYPE
        );
        assert_eq!(
            pair.binding().resource_type().as_str(),
            SECURITY_KEY_BINDING_RESOURCE_TYPE
        );
    }

    /// Canonical minimal base acceptance without `spec.provider`.
    #[test]
    fn the_canonical_minimal_base_is_accepted_without_a_provider_extension() {
        assert_minimal_base_round_trips(contract().service(), r#"{"mode":"authority"}"#);
        assert_minimal_base_round_trips(
            contract().binding(),
            r#"{"serviceRef":"security-key.d2bus.org.SecurityKeyService/yubikey-primary","target":{"guestRef":"Guest/corp-vm","userRef":"User/alice"}}"#,
        );
    }

    /// Each initial and fake alternate Provider passes the identical base
    /// conformance fixture.
    #[test]
    fn every_implementation_passes_the_identical_base_fixture() {
        assert_base_is_provider_neutral(
            contract(),
            r#"{"mode":"authority"}"#,
            r#"{"serviceRef":"security-key.d2bus.org.SecurityKeyService/yubikey-primary","target":{"guestRef":"Guest/corp-vm","userRef":"User/alice"}}"#,
            "device-security-key",
            "security-key-alternate",
        );
    }

    /// Implementation-detail rejection: the physical device selector is not a
    /// semantic base field for this family.
    #[test]
    fn the_physical_device_selector_is_not_a_base_field() {
        assert_eq!(
            contract()
                .service()
                .spec()
                .validate_names(["providerRef", "mode", "deviceRef"]),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// Status-only observations: attachment state is observed, never desired.
    #[test]
    fn attachment_is_a_status_field_and_not_a_binding_spec_field() {
        assert!(
            contract()
                .binding()
                .status()
                .validate_names(["attachment"])
                .is_ok()
        );
        assert_eq!(
            contract().binding().spec().validate_names([
                "providerRef",
                "serviceRef",
                "target",
                "attachment"
            ]),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// A Core projection rejects `spec.provider` and the authority descriptor.
    #[test]
    fn a_projection_rejects_a_provider_extension_and_the_authority_descriptor() {
        let spec = crate::v3::resource::ResourceSpec::new(
            Some(provider_ref("device-security-key")),
            None,
            object(r#"{"authority":{},"mode":"projection"}"#),
            None,
        )
        .unwrap();
        assert_eq!(
            contract().projection().validate_projection_spec(&spec),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// The backing declaration is determinate and empty, so the derived
    /// factory is constructible and denies every backing reference.
    #[test]
    fn the_backing_declaration_is_no_backing_and_the_factory_is_constructible() {
        assert!(
            contract()
                .projection()
                .allowed_backing_ref_types()
                .is_empty()
        );
        assert!(contract().projection().projection_factory().is_ok());
    }

    /// The empty allowlist is a deny-all, not an unconstrained value.
    #[test]
    fn the_empty_backing_set_admits_no_backing_reference() {
        let factory = contract().projection().projection_factory().unwrap();
        for resource_type in ["Device", "Endpoint", SECURITY_KEY_SERVICE_RESOURCE_TYPE] {
            assert_eq!(
                factory.admits_backing_ref(&resource_envelope(resource_type, None)),
                Err(crate::v3::ProviderContractError::ProjectionFactoryInvalid)
            );
        }
        let usb = super::super::usb::contract()
            .projection()
            .projection_factory()
            .unwrap();
        assert!(
            usb.allowed_backing_ref_types()
                .iter()
                .any(|name| name.as_str() == "Device")
        );
    }
}
