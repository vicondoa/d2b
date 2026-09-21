//! The USB Service/Binding resource driver: the v3 `ResourceDriver`
//! conversion of the daemon-owned USBIP Provider path.
//!
//! The family serves the two converted USB ResourceTypes the Provider owns -
//! `usb.d2bus.org.UsbService` and `usb.d2bus.org.UsbBinding` - and their
//! `Device` rows belong to the `d2b-provider-device` family, which owns the
//! `Device` ResourceType. The scope here is the crate's own declaration: the
//! rows below name the components this crate serves, and the typed effect
//! behind [`UsbipDriverEffects`] dispatches on that declaration rather than on
//! a type-name heuristic.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> the [`ProviderRow`] registrations under the family's
//!   ResourceTypes.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`]: Binding rows adopt when their
//!   declared child set is committed; the Service row realizes nothing
//!   through resource rows and adopts in the effect.
//! - `plan`/`reconcile`/`execute_effect` -> [`ResourceDriver::reconcile`]:
//!   the Binding's Provider-declared children are committed through the
//!   manager child API before the typed effect runs (F1).
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_provider::v3::semantic_services::child_resources::{
    BindingChildIntent, BindingChildSet,
};
use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ZoneId};
use d2b_provider_toolkit::{
    ProviderRow, SharedProviderDeclarationError, SharedProviderDriverArgs,
    SharedProviderDriverFactory, SharedProviderEffectError, SharedProviderEffectOutcome,
    SharedProviderEffectRequest, SharedProviderFamily, SharedProviderFinalize, key_ref,
    shared_provider_spec_decoder,
};
use d2b_resource_runtime::context::{ChildEnsure, ResourceContext};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_types::{AllowedSources, DriverDescriptor, ServiceDecl, WellKnownType};
use serde_json::{Value, json};

use crate::effects_service::USBIP_EFFECTS_SERVICE;

pub use crate::{PROVIDER_REF, USB_BINDING_RESOURCE_TYPE, USB_SERVICE_RESOURCE_TYPE};

/// The controller reference the Service row's effects bind.
pub const USBIP_SERVICE_CONTROLLER_REF: &str = "Process/device-usbip-service-controller";

/// The controller reference the Binding row's effects bind.
pub const USBIP_BINDING_CONTROLLER_REF: &str = "Process/device-usbip-binding-controller";

/// Preserved self-resync for the USB rows (old shared Runner repair interval).
pub const USBIP_RESYNC: Duration = Duration::from_secs(30);

/// The USB family's closed component vocabulary.
///
/// The `Device` component of this Provider is served by the `Device` type's
/// own family (`d2b-provider-device`); this crate declares the two
/// ResourceTypes it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipComponent {
    /// The USB authority Service (`usb.d2bus.org.UsbService`).
    Service,
    /// The per-Guest USB Binding (`usb.d2bus.org.UsbBinding`).
    Binding,
}

/// The rows this family declares, in the preserved registration order.
pub const USBIP_REGISTRATIONS: [ProviderRow<UsbipComponent>; 2] = [
    ProviderRow {
        resource_type: USB_SERVICE_RESOURCE_TYPE,
        component: UsbipComponent::Service,
        controller_ref: USBIP_SERVICE_CONTROLLER_REF,
        provider_ref: PROVIDER_REF,
        effect_id: "device-usbip-service",
        resync: USBIP_RESYNC,
    },
    ProviderRow {
        resource_type: USB_BINDING_RESOURCE_TYPE,
        component: UsbipComponent::Binding,
        controller_ref: USBIP_BINDING_CONTROLLER_REF,
        provider_ref: PROVIDER_REF,
        effect_id: "device-usbip-binding",
        resync: USBIP_RESYNC,
    },
];

/// The Provider effect surface the USB driver needs.
///
/// The production implementation owns the preserved USBIP supervisor, the
/// zone-wide authority ledger, and the daemon/broker dispatcher; test doubles
/// implement the same seam.
#[async_trait]
pub trait UsbipDriverEffects: Send + Sync + 'static {
    /// Reconcile one USB Service or Binding row through its typed lifecycle
    /// controller.
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one USB row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

/// Everything the composition must construct to instantiate the USB driver
/// factory for one zone.
pub struct UsbipDriverArgs {
    /// The zone the driver serves.
    pub zone: String,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The daemon-supplied facet set the family's own effects
    /// implementation is built from (U12 usbip step): the driver never
    /// receives a daemon-built effect port (R2).
    pub facets: crate::facets::UsbipEffectFacets,
}

/// The family's declarations and typed Provider effect.
struct UsbipFamily {
    effects: Arc<dyn UsbipDriverEffects>,
}

#[async_trait]
impl SharedProviderFamily for UsbipFamily {
    type Component = UsbipComponent;
    type State = ();

    fn rows(&self) -> &'static [ProviderRow<Self::Component>] {
        &USBIP_REGISTRATIONS
    }

    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        component: UsbipComponent,
        spec: &Value,
    ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError> {
        match component {
            // The Service realizes its supervisor through the typed effect;
            // it declares no manager child of its own.
            UsbipComponent::Service => Ok(None),
            UsbipComponent::Binding => {
                let owner = key_ref(ctx.key());
                let zone = ZoneId::parse(ctx.key().zone.clone())
                    .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                let service_ref = spec_ref(spec, "/spec/serviceRef")?;
                let guest_ref = spec_ref(spec, "/spec/guestRef")?;
                let desired = crate::binding_child_resources(&owner, &service_ref, &guest_ref)
                    .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                binding_child_ensures(&desired, &zone)
            }
        }
    }

    fn declared_dependency_refs(
        &self,
        component: UsbipComponent,
        spec: &Value,
        metadata: &Value,
    ) -> Vec<ResourceRef> {
        declared_dependency_refs(component, spec, metadata)
    }

    async fn effect(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
        _state: &(),
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.effects.reconcile_usbip(component, request).await
    }

    async fn finalize(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
        _state: &(),
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.effects.finalize(component, request).await
    }
}

/// The resource verbs the USB types support.
const USBIP_VERBS: &[&str] = &[
    "get",
    "list",
    "watch",
    "create",
    "update-spec",
    "update-status",
    "update-metadata",
    "update-finalizers",
    "delete",
];

/// The execution domains the USB types can be reconciled in.
const USBIP_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the USB Service reads while reconciling: the backing
/// Device it claims and the Network whose relay it owns.
const USBIP_SERVICE_READS: &[WellKnownType] = &[WellKnownType::DEVICE, WellKnownType::NETWORK];

/// The resource types the USB Binding reads while reconciling: the Service it
/// attaches and the Guest it attaches to.
const USBIP_BINDING_READS: &[WellKnownType] = &[WellKnownType::USB_SERVICE, WellKnownType::GUEST];

/// The USB family's driver declarations.
///
/// Both types are `BUILTIN | STARTUP | RUNTIME` (the RUNTIME bit is present):
/// USB hardware presence is host-dependent, so the driver may arrive late.
/// The Service is a qualified semantic Service and is therefore exportable;
/// a Binding is not. Neither type serves broker operations or creates
/// children through this declaration beyond the Binding's declared child set.
/// The family's declared effects service rides on the Service descriptor
/// alone (U8):the family hosts one effects service per zone; a Binding
/// descriptor declares none.
pub fn usbip_descriptors(args: UsbipDriverArgs) -> [DriverDescriptor; 2] {
    let factory: Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory> =
        Arc::new(SharedProviderDriverFactory::new(SharedProviderDriverArgs {
            zone: args.zone,
            controller_generation: args.controller_generation,
            family: Arc::new(UsbipFamily {
                effects: Arc::new(crate::effects_service::UsbipEffects::new(args.facets)),
            }),
        }));
    let descriptor = |resource_type: WellKnownType,
                      exportable: bool,
                      reads: &'static [WellKnownType],
                      services: &'static [ServiceDecl]| DriverDescriptor {
        resource_type,
        allowed_sources: AllowedSources::BUILTIN
            | AllowedSources::STARTUP
            | AllowedSources::RUNTIME,
        verbs: USBIP_VERBS,
        execution: USBIP_EXECUTION_DOMAINS,
        exportable,
        reads,
        operations: &[],
        creations: &[],
        startup: &[],
        services,
        decoder: shared_provider_spec_decoder(),
        factory: Arc::clone(&factory),
    };
    [
        descriptor(
            WellKnownType::USB_SERVICE,
            true,
            USBIP_SERVICE_READS,
            &[USBIP_EFFECTS_SERVICE],
        ),
        descriptor(
            WellKnownType::USB_BINDING,
            false,
            USBIP_BINDING_READS,
            &[],
        ),
    ]
}

/// The dependency references one USB row declares, in the order the old
/// dependency selectors carried them: the Service reads its backing Device,
/// the Binding its Service and Guest.
pub fn declared_dependency_refs(
    component: UsbipComponent,
    spec: &Value,
    _metadata: &Value,
) -> Vec<ResourceRef> {
    let mut refs = Vec::new();
    match component {
        UsbipComponent::Service => {
            if let Ok(reference) = spec_ref(spec, "/spec/backingDeviceRef") {
                refs.push(reference);
            }
        }
        UsbipComponent::Binding => {
            if let Ok(reference) = spec_ref(spec, "/spec/serviceRef") {
                refs.push(reference);
            }
            if let Ok(reference) = spec_ref(spec, "/spec/guestRef") {
                refs.push(reference);
            }
        }
    }
    refs
}

/// One reference field of a stored spec.
fn spec_ref(spec: &Value, path: &str) -> Result<ResourceRef, SharedProviderDeclarationError> {
    spec.pointer(path)
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or(SharedProviderDeclarationError::SpecInvalid)
}

/// One Provider-declared Binding child as a manager child row.
///
/// Old Core `materialize_child_create_payload`: Providers declare intent, Core
/// owns the child body, and the Process Provider stays Core-chosen.
fn binding_child_ensure(
    intent: &BindingChildIntent,
    zone: &ZoneId,
) -> Result<ChildEnsure, SharedProviderDeclarationError> {
    let invalid = || SharedProviderDeclarationError::SpecInvalid;
    let payload =
        d2b_core_controller::materialize_child_create_payload(intent, zone).map_err(|_| invalid())?;
    let value = serde_json::from_slice::<Value>(&payload).map_err(|_| invalid())?;
    let spec = value.get("spec").cloned().ok_or_else(invalid)?;
    let metadata = json!({
        "ownerRef": intent.owner_ref().to_canonical_string(),
        "labels": {},
        "annotations": {},
    });
    Ok(ChildEnsure {
        type_name: ResourceTypeName::new(intent.kind().resource_type()),
        name: intent.resource_ref().name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec).map_err(|_| invalid())?,
        metadata: serde_json::to_vec(&metadata).map_err(|_| invalid())?,
    })
}

/// Materialize one Provider-declared Binding child set into manager rows.
fn binding_child_ensures(
    desired: &BindingChildSet,
    zone: &ZoneId,
) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError> {
    desired
        .iter()
        .map(|intent| binding_child_ensure(intent, zone))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        PROVIDER_REF, USBIP_REGISTRATIONS, USB_BINDING_RESOURCE_TYPE, USB_SERVICE_RESOURCE_TYPE,
        UsbipComponent, UsbipDriverArgs,
        usbip_descriptors,
    };

    fn descriptors() -> [d2b_resource_types::DriverDescriptor; 2] {
        usbip_descriptors(UsbipDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
                .expect("generation"),
            facets: crate::test_support::recording_facets(Arc::new(
                crate::test_support::RecordingRuntime::default(),
            )),
        })
    }

    /// The declaration serves exactly the two converted USB ResourceTypes,
    /// over one factory the registry can register each type with.
    #[test]
    fn descriptors_declare_the_usb_types() {
        let descriptors = descriptors();
        let types = descriptors
            .iter()
            .map(|descriptor| descriptor.resource_type.to_resource_type_name().as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                USB_SERVICE_RESOURCE_TYPE.to_owned(),
                USB_BINDING_RESOURCE_TYPE.to_owned()
            ]
        );
        assert!(descriptors[0].exportable, "a USB Service is exportable");
        assert!(!descriptors[1].exportable, "a USB Binding is not");
        for descriptor in &descriptors {
            let factory_types = descriptor
                .factory
                .resource_types()
                .iter()
                .map(|resource_type| resource_type.as_str().to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                factory_types,
                vec![
                    USB_SERVICE_RESOURCE_TYPE.to_owned(),
                    USB_BINDING_RESOURCE_TYPE.to_owned()
                ],
                "the shared factory serves the family's own type set"
            );
        }
    }

    /// The rows carry the preserved identities and resync cadence.
    #[test]
    fn rows_keep_the_preserved_identity() {
        assert_eq!(USBIP_REGISTRATIONS[0].resource_type, USB_SERVICE_RESOURCE_TYPE);
        assert_eq!(USBIP_REGISTRATIONS[0].component, UsbipComponent::Service);
        assert_eq!(USBIP_REGISTRATIONS[0].effect_id, "device-usbip-service");
        assert_eq!(USBIP_REGISTRATIONS[1].resource_type, USB_BINDING_RESOURCE_TYPE);
        assert_eq!(USBIP_REGISTRATIONS[1].component, UsbipComponent::Binding);
        assert_eq!(USBIP_REGISTRATIONS[1].effect_id, "device-usbip-binding");
        for row in &USBIP_REGISTRATIONS {
            assert_eq!(row.provider_ref, PROVIDER_REF);
            assert_eq!(row.resync, super::USBIP_RESYNC);
        }
    }
}
