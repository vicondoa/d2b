//! The shared USB semantic Service and Binding base contract.
//!
//! This module owns the common base spec, status, and schema contract for the
//! frozen USB pair `usb.d2bus.org.UsbService` and `usb.d2bus.org.UsbBinding`.
//! The field sets below are the top-level provider-neutral base fields stated
//! by the USBIP Provider dossier's owner authority, projection, and per-Guest
//! Binding sections, which describe the base as carrying only generic
//! whole-device semantics.
//!
//! Network selection, transport topology, host module and backend policy,
//! proxy behaviour, firewall policy, busid handling, and transport tuning are
//! never base fields. No raw busid, sysfs path, interface, address, port, file
//! descriptor, or credential appears in either base spec.
//!
//! Interiors this catalog does not model. `accessPolicy`, `backingAuthority`,
//! and `attachmentPolicy` appear in the dossier's examples with member names,
//! but their closed value domains are not stated as a frozen table, so this
//! catalog freezes only the top-level base field set.
//!
//! Status field names this catalog could not determine. The Binding's
//! `attachmentPhase` is stated as a spelling; the observed Service generation,
//! bounded queue position, last closed error, and attach and detach timestamps
//! are described without frozen names, as is the projection Service's import
//! lease and remote generation state. The common status layers below carry
//! only the stated spellings and reject the rest.

use std::sync::OnceLock;

use super::{
    super::provider::BindingTargetType, NonEmpty, SemanticBackingDeclaration, SemanticFamily,
    SemanticPairContract, SemanticPairDeclaration,
};

/// The dot-qualified API ResourceType of the USB owner authority and consumer
/// projection Service.
pub const USB_SERVICE_RESOURCE_TYPE: &str = "usb.d2bus.org.UsbService";

/// The dot-qualified API ResourceType of the USB local consumer intent
/// Binding.
pub const USB_BINDING_RESOURCE_TYPE: &str = "usb.d2bus.org.UsbBinding";

const SERVICE_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "mode",
    "backingDeviceRef",
    "accessPolicy",
    "backingAuthority",
    "sourceSchemaFingerprint",
];
const SERVICE_SPEC_REQUIRED: &[&str] = &["providerRef", "mode", "accessPolicy"];
const SERVICE_STATUS_ALLOWED: &[&str] = &["access", "backingAuthority"];

const BINDING_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "updatePolicy",
    "serviceRef",
    "guestRef",
    "accessPolicy",
    "attachmentPolicy",
];
const BINDING_SPEC_REQUIRED: &[&str] = &[
    "providerRef",
    "serviceRef",
    "guestRef",
    "accessPolicy",
    "attachmentPolicy",
];
const BINDING_STATUS_ALLOWED: &[&str] = &["attachmentPhase"];

/// Projection mode forbids the backing Device reference, backing-authority
/// ownership, and any local physical effect. It permits only `providerRef`,
/// the semantic base and import fields, and ResourceImport ownership.
const PROJECTION_SPEC_ALLOWED: &[&str] = &[
    "providerRef",
    "mode",
    "accessPolicy",
    "sourceSchemaFingerprint",
];
const PROJECTION_SPEC_REQUIRED: &[&str] = &["providerRef", "mode", "accessPolicy"];

/// A USB Binding is per-Guest.
const ALLOWED_BINDING_TARGET_REF_TYPES: &[BindingTargetType] = &[BindingTargetType::Guest];

const DECLARATION: SemanticPairDeclaration = SemanticPairDeclaration {
    family: SemanticFamily::Usb,
    service_type_segment: "UsbService",
    binding_type_segment: "UsbBinding",
    service_spec_allowed: SERVICE_SPEC_ALLOWED,
    service_spec_required: SERVICE_SPEC_REQUIRED,
    service_status_allowed: SERVICE_STATUS_ALLOWED,
    binding_spec_allowed: BINDING_SPEC_ALLOWED,
    binding_spec_required: BINDING_SPEC_REQUIRED,
    binding_status_allowed: BINDING_STATUS_ALLOWED,
    backing: SemanticBackingDeclaration::Constrained {
        types: NonEmpty::of("Device"),
        fields: NonEmpty::of("backingDeviceRef"),
    },
    allowed_binding_target_ref_types: ALLOWED_BINDING_TARGET_REF_TYPES,
    projection_spec_allowed: PROJECTION_SPEC_ALLOWED,
    projection_spec_required: PROJECTION_SPEC_REQUIRED,
};

/// Borrow the frozen USB pair contract.
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

    const MINIMAL_SERVICE: &str = r#"{"accessPolicy":{},"mode":"authority"}"#;
    const MINIMAL_BINDING: &str = r#"{"accessPolicy":{},"attachmentPolicy":{},"guestRef":"Guest/corp-vm","serviceRef":"usb.d2bus.org.UsbService/work-token"}"#;

    #[test]
    fn the_pair_names_the_exact_frozen_resource_types() {
        let pair = contract();
        assert_eq!(
            pair.service().resource_type().as_str(),
            USB_SERVICE_RESOURCE_TYPE
        );
        assert_eq!(
            pair.binding().resource_type().as_str(),
            USB_BINDING_RESOURCE_TYPE
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
            "device-usbip",
            "usb-alternate",
        );
    }

    /// Implementation-detail rejection: no locator or transport detail is a
    /// base field.
    #[test]
    fn no_locator_or_transport_detail_is_a_base_field() {
        for detail in ["busid", "networkRef", "relayEndpointRef", "sysfsPath"] {
            assert_eq!(
                contract().service().spec().validate_names([
                    "providerRef",
                    "mode",
                    "accessPolicy",
                    detail
                ]),
                Err(SemanticContractError::SchemaViolation)
            );
        }
    }

    /// Owner versus projection discrimination: a projection forbids the
    /// backing Device reference and backing-authority ownership.
    #[test]
    fn a_projection_forbids_the_backing_device_and_authority() {
        for extra in [
            r#""backingDeviceRef":"Device/work-token""#,
            r#""backingAuthority":{}"#,
        ] {
            let base = format!(r#"{{"accessPolicy":{{}},"mode":"projection",{extra}}}"#);
            let spec = crate::v3::resource::ResourceSpec::new(
                Some(provider_ref("device-usbip")),
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

    /// Status-only observations: the attachment phase is observed, never
    /// smuggled into Binding spec.
    #[test]
    fn the_attachment_phase_is_status_only() {
        assert!(
            contract()
                .binding()
                .status()
                .validate_names(["attachmentPhase"])
                .is_ok()
        );
        assert_eq!(
            contract().binding().spec().validate_names([
                "providerRef",
                "serviceRef",
                "guestRef",
                "accessPolicy",
                "attachmentPolicy",
                "attachmentPhase",
            ]),
            Err(SemanticContractError::SchemaViolation)
        );
    }

    /// The projection factory type binding is derivable for this family.
    #[test]
    fn the_projection_factory_binds_the_frozen_types() {
        let factory = contract().projection().projection_factory().unwrap();
        assert_eq!(factory.service_type().as_str(), USB_SERVICE_RESOURCE_TYPE);
        assert_eq!(factory.binding_type().as_str(), USB_BINDING_RESOURCE_TYPE);
        assert!(
            factory
                .allowed_backing_ref_types()
                .iter()
                .any(|name| name.as_str() == "Device")
        );
    }
}
