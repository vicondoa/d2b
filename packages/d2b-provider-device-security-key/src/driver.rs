//! The SecurityKey Service/Binding resource driver: the v3 `ResourceDriver`
//! conversion of the daemon-owned security-key Provider path.
//!
//! The family serves the two converted security-key ResourceTypes the
//! Provider owns - `security-key. d2bus. org.SecurityKeyService` and
//! `security-key. d2bus. org.SecurityKeyBinding` - and their `Device` rows
//! belong to the `d2b-provider-device` family, which owns the `Device`
//! ResourceType.
//!
//! The Service's relay is a declared creation: the Host relay Process and the
//! relay Endpoint it produces are [`ChildCreation`] rows naming the provider
//! crates' exported identities, and the driver derives the same two rows from
//! the Service's own declared device reference on every pass.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> the [`ProviderRow`] registrations under the family's
//!   ResourceTypes.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`]: rows adopt when their declared
//!   child set is committed.
//! - `plan`/`reconcile`/`execute_effect` -> [`ResourceDriver::reconcile`]:
//!   the declared child set is committed through the manager child API before
//!   the typed effect runs (F1).
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_provider::v3::semantic_services::child_resources::{
    BindingChildIntent, BindingChildSet,
};
use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
use d2b_provider_toolkit::{
    HOST_REF, ProviderRow, SharedProviderDeclarationError, SharedProviderDriverArgs,
    SharedProviderDriverFactory, SharedProviderEffectError, SharedProviderEffectOutcome,
    SharedProviderEffectRequest, SharedProviderFamily, SharedProviderFinalize, key_ref,
    resource_uid, shared_provider_spec_decoder,
};
use d2b_resource_runtime::context::{ChildEnsure, ResourceContext};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, ChildCustody, DriverDescriptor,
    ServiceDecl, WellKnownType,
};

use crate::effects_service::SECURITY_KEY_EFFECTS_SERVICE;
use serde_json::{Value, json};

pub use crate::{PROVIDER_REF, SECURITY_KEY_BINDING_RESOURCE_TYPE, SECURITY_KEY_SERVICE_RESOURCE_TYPE};

/// The controller reference the Service row's effects bind.
pub const SECURITY_KEY_SERVICE_CONTROLLER_REF: &str =
    "Process/device-security-key-service-controller";

/// The controller reference the Binding row's effects bind.
pub const SECURITY_KEY_BINDING_CONTROLLER_REF: &str =
    "Process/device-security-key-binding-controller";

/// Preserved self-resync for the security-key rows (old shared Runner repair
/// interval).
pub const SECURITY_KEY_RESYNC: Duration = Duration::from_secs(30);

/// The Process Provider that realizes the Host relay and the in-guest
/// frontend.
const RELAY_PROCESS_PROVIDER_REF: &str = d2b_provider_process_minijail::PROVIDER_REF;

/// The Process Provider that realizes the binding's in-guest frontend.
const FRONTEND_PROCESS_PROVIDER_REF: &str = d2b_provider_process_systemd::PROVIDER_REF;

/// The security-key family's closed component vocabulary.
///
/// The `Device` component of this Provider is served by the `Device` type's
/// own family (`d2b-provider-device`); this crate declares the two
/// ResourceTypes it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityKeyComponent {
    /// The security-key authority Service
    /// (`security-key. d2bus. org.SecurityKeyService`).
    Service,
    /// The per-Guest security-key Binding
    /// (`security-key. d2bus. org.SecurityKeyBinding`).
    Binding,
}

/// The rows this family declares, in the preserved registration order.
pub const SECURITY_KEY_REGISTRATIONS: [ProviderRow<SecurityKeyComponent>; 2] = [
    ProviderRow {
        resource_type: SECURITY_KEY_SERVICE_RESOURCE_TYPE,
        component: SecurityKeyComponent::Service,
        controller_ref: SECURITY_KEY_SERVICE_CONTROLLER_REF,
        provider_ref: PROVIDER_REF,
        effect_id: "device-security-key-service",
        resync: SECURITY_KEY_RESYNC,
    },
    ProviderRow {
        resource_type: SECURITY_KEY_BINDING_RESOURCE_TYPE,
        component: SecurityKeyComponent::Binding,
        controller_ref: SECURITY_KEY_BINDING_CONTROLLER_REF,
        provider_ref: PROVIDER_REF,
        effect_id: "device-security-key-binding",
        resync: SECURITY_KEY_RESYNC,
    },
];

/// The Service's declared relay creation: the Host relay Process and the
/// relay Endpoint it produces.
///
/// The Endpoint is owned by this Provider; the Process execution is delegated
/// to the fixed system Process Provider, exactly as the old inline relay spec
/// declared it.
pub const SECURITY_KEY_SERVICE_CREATIONS: [ChildCreation; 2] = [
    ChildCreation {
        child: WellKnownType::PROCESS,
        provider_ref: RELAY_PROCESS_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 0,
    },
    ChildCreation {
        child: WellKnownType::ENDPOINT,
        provider_ref: PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 1,
    },
];

/// The Binding's declared creation: the in-guest frontend Process and the
/// guest Endpoint it produces.
pub const SECURITY_KEY_BINDING_CREATIONS: [ChildCreation; 2] = [
    ChildCreation {
        child: WellKnownType::PROCESS,
        provider_ref: FRONTEND_PROCESS_PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 0,
    },
    ChildCreation {
        child: WellKnownType::ENDPOINT,
        provider_ref: PROVIDER_REF,
        custody: ChildCustody::DriverOwned,
        order: 1,
    },
];

/// The Provider effect surface the security-key driver needs.
///
/// The production implementation owns the preserved lease, the host relay
/// service, and the broker-backed hidraw effect; test doubles implement the
/// same seam.
#[async_trait]
pub trait SecurityKeyDriverEffects: Send + Sync + 'static {
    /// Reconcile one security-key Service or Binding row through its typed
    /// lifecycle controller.
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one security-key row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

/// Everything the composition must construct to instantiate the security-key
/// driver factory for one zone.
pub struct SecurityKeyDriverArgs {
    /// The zone the driver serves.
    pub zone: ZoneId,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The daemon-supplied facet set the family's own effects
    /// implementation is built from (U12 security-key step): the driver
    /// never receives a daemon-built effect port (R2).
    pub facets: crate::facets::SecurityKeyEffectFacets,
}

/// The family's declarations and typed Provider effect.
struct SecurityKeyFamily {
    effects: Arc<dyn SecurityKeyDriverEffects>,
}

#[async_trait]
impl SharedProviderFamily for SecurityKeyFamily {
    type Component = SecurityKeyComponent;
    type State = ();

    fn rows(&self) -> &'static [ProviderRow<Self::Component>] {
        &SECURITY_KEY_REGISTRATIONS
    }

    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        component: SecurityKeyComponent,
        spec: &Value,
    ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDeclarationError> {
        let owner =
            key_ref(ctx.key()).map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
        match component {
            SecurityKeyComponent::Service => {
                let settings = spec
                    .pointer("/spec/provider/settings")
                    .ok_or(SharedProviderDeclarationError::SpecInvalid)?;
                let device_ref = settings
                    .get("deviceRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderDeclarationError::SpecInvalid)?;
                let relay_endpoint_ref = settings
                    .get("relayEndpointRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderDeclarationError::SpecInvalid)?;
                let device_key = ResourceKey::new(
                    ctx.key().zone.as_str(),
                    device_ref.resource_type().as_str(),
                    device_ref.name().as_str(),
                );
                let device_uid = ctx
                    .get(&device_key)
                    .await
                    .map_err(|_| SharedProviderDeclarationError::ChildMutation)?
                    .filter(|row| !row.deleting)
                    .map(|row| row.uid);
                let Some(device_uid) = device_uid else {
                    // The Device row the relay is derived from is not present:
                    // the Service effect reports Pending until it is; no child
                    // is declared yet (old effect returned Pending here).
                    return Ok(Some(Vec::new()));
                };
                let device_uid = resource_uid(&device_uid)
                    .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                security_key_relay_child_ensures(&owner, &device_ref, &relay_endpoint_ref, &device_uid)
                    .map(Some)
            }
            SecurityKeyComponent::Binding => {
                let zone = ZoneId::parse(ctx.key().zone.clone())
                    .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                let service_ref = spec_ref(spec, "/spec/serviceRef")?;
                let target_ref = spec
                    .pointer("/spec/target/guestRef")
                    .or_else(|| spec.pointer("/spec/guestRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or(SharedProviderDeclarationError::SpecInvalid)?;
                let user_ref = spec
                    .pointer("/spec/target/userRef")
                    .or_else(|| spec.pointer("/spec/userRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok());
                let desired = match user_ref {
                    Some(user_ref) => crate::SecurityKeyController::child_resources_for_user(
                        &owner,
                        &service_ref,
                        &target_ref,
                        &user_ref,
                    ),
                    None => crate::SecurityKeyController::child_resources(
                        &owner,
                        &service_ref,
                        &target_ref,
                    ),
                }
                .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
                binding_child_ensures(&desired, &zone)
            }
        }
    }

    fn declared_dependency_refs(
        &self,
        component: SecurityKeyComponent,
        spec: &Value,
        metadata: &Value,
    ) -> Vec<ResourceRef> {
        declared_dependency_refs(component, spec, metadata)
    }

    async fn effect(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
        _state: &(),
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.effects.reconcile_security_key(component, request).await
    }

    async fn finalize(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
        _state: &(),
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.effects.finalize(component, request).await
    }
}

/// The execution domains the security-key types can be reconciled in.
const SECURITY_KEY_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the security-key Service reads while reconciling: the
/// backing Device it claims and the relay Endpoint it produces.
const SECURITY_KEY_SERVICE_READS: &[WellKnownType] = &[WellKnownType::DEVICE];

/// The resource types the security-key Binding reads while reconciling: the
/// Service it attaches and the Guest it attaches to.
const SECURITY_KEY_BINDING_READS: &[WellKnownType] =
    &[WellKnownType::SECURITY_KEY_SERVICE, WellKnownType::GUEST];

/// The security-key family's driver declarations.
///
/// Both types are `BUILTIN | STARTUP | RUNTIME` (the RUNTIME bit is present):
/// security-key hardware presence is host-dependent, so the driver may arrive
/// late. The Service is a qualified semantic Service and is therefore
/// exportable; a Binding is not. The Service's relay and the Binding's
/// frontend are declared in [`SECURITY_KEY_SERVICE_CREATIONS`]and
/// [`SECURITY_KEY_BINDING_CREATIONS`]. The family's declared effects
/// service rides on the Service descriptor alone (U8):the family
/// hosts one effects service per zone; a Binding descriptor declares
/// none.
pub fn security_key_descriptors(args: SecurityKeyDriverArgs) -> [DriverDescriptor; 2] {
    let factory: Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory> =
        Arc::new(SharedProviderDriverFactory::new(SharedProviderDriverArgs {
            zone: args.zone,
            controller_generation: args.controller_generation,
            family: Arc::new(SecurityKeyFamily {
                effects: Arc::new(crate::effects_service::SecurityKeyEffects::new(args.facets)),
            }),
        }));
    let descriptor = |resource_type: WellKnownType,
                      exportable: bool,
                      reads: &'static [WellKnownType],
                      creations: &'static [ChildCreation],
                      services: &'static [ServiceDecl]| DriverDescriptor {
        resource_type,
        allowed_sources: AllowedSources::BUILTIN
            | AllowedSources::STARTUP
            | AllowedSources::RUNTIME,
        verbs: CONVERTED_TYPE_VERBS,
        execution: SECURITY_KEY_EXECUTION_DOMAINS,
        exportable,
        reads,
        operations: &[],
        creations,
        startup: &[],
        services,
        decoder: shared_provider_spec_decoder(),
        factory: Arc::clone(&factory),
    };
    [
        descriptor(
            WellKnownType::SECURITY_KEY_SERVICE,
            true,
            SECURITY_KEY_SERVICE_READS,
            &SECURITY_KEY_SERVICE_CREATIONS,
            &[SECURITY_KEY_EFFECTS_SERVICE],
        ),
        descriptor(
            WellKnownType::SECURITY_KEY_BINDING,
            false,
            SECURITY_KEY_BINDING_READS,
            &SECURITY_KEY_BINDING_CREATIONS,
            &[],
        ),
    ]
}

/// The dependency references one security-key row declares, in the order the
/// old dependency selectors carried them: the Service reads its Device and
/// relay Endpoint, the Binding its Service and Guest.
pub fn declared_dependency_refs(
    component: SecurityKeyComponent,
    spec: &Value,
    _metadata: &Value,
) -> Vec<ResourceRef> {
    match component {
        SecurityKeyComponent::Service => [
            spec.pointer("/spec/provider/settings/deviceRef")
                .and_then(Value::as_str)
                .and_then(|value| ResourceRef::parse(value).ok()),
            spec.pointer("/spec/provider/settings/relayEndpointRef")
                .and_then(Value::as_str)
                .and_then(|value| ResourceRef::parse(value).ok()),
        ]
        .into_iter()
        .flatten()
        .collect(),
        SecurityKeyComponent::Binding => [
            spec_ref(spec, "/spec/serviceRef").ok(),
            spec.pointer("/spec/target/guestRef")
                .or_else(|| spec.pointer("/spec/guestRef"))
                .and_then(Value::as_str)
                .and_then(|value| ResourceRef::parse(value).ok()),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

/// One reference field of a stored spec.
fn spec_ref(spec: &Value, path: &str) -> Result<ResourceRef, SharedProviderDeclarationError> {
    spec.pointer(path)
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or(SharedProviderDeclarationError::SpecInvalid)
}

/// The Service's desired children: the Host relay Process and the relay
/// Endpoint it produces (old inline specs in `reconcile_security_key`,
/// unchanged).
fn security_key_relay_child_ensures(
    owner: &ResourceRef,
    device_ref: &ResourceRef,
    relay_endpoint_ref: &ResourceRef,
    device_uid: &ResourceUid,
) -> Result<Vec<ChildEnsure>, SharedProviderDeclarationError> {
    let invalid = || SharedProviderDeclarationError::SpecInvalid;
    let relay_process_name = crate::security_key_process_name(
        device_uid,
        crate::SecurityKeyProcessRole::HostRelay,
    )
    .map_err(|_| invalid())?;
    let relay_process_ref = ResourceRef::parse(&format!("Process/{relay_process_name}"))
        .map_err(|_| invalid())?;
    let metadata = serde_json::to_vec(&json!({
        "ownerRef": owner.to_canonical_string(),
        "labels": {},
        "annotations": {},
    }))
    .map_err(|_| invalid())?;
    let relay_process_spec = json!({
        "providerRef": RELAY_PROCESS_PROVIDER_REF,
        "executionRef": HOST_REF,
        "domain": "system",
        "processClass": "service",
        "template": "sk-relay",
        "desiredLifecycle": "running",
        "deviceUsage": [{
            "deviceRef": device_ref.to_canonical_string(),
            "access": "exclusive",
            "purpose": "hidraw-fido"
        }],
        "sandbox": {
            "namespaceClasses": ["mount", "ipc", "pid"],
            "capabilityClasses": [],
            "seccompClass": "sk-relay",
            "environmentClass": "provider-defined",
            "startRoot": false,
            "noNewPrivileges": true,
            "readOnlyRoot": true
        },
        "budget": {
            "pids": {"limit": 32},
            "fds": {"limit": 64},
            "memory": {"limit": "32Mi"}
        }
    });
    let endpoint_spec = json!({
        "providerRef": PROVIDER_REF,
        "producerRef": relay_process_ref.to_canonical_string(),
        "endpointClass": "device",
        "transport": "vsock",
        "purpose": "security-key-ctaphid-relay",
        "serviceFingerprint": "device-security-key.d2bus.org/SecurityKeyCtapRelay.v3",
        "locality": "cross-domain",
        "visibility": "zone",
        "attachmentPolicy": {
            "supported": true,
            "maxAttachments": 1
        },
        "consumerPolicy": {
            "allowedProviderComponents": ["device-security-key"],
            "allowedOperations": ["resolve"]
        },
        "lifecyclePolicy": "recycle-with-producer"
    });
    Ok(vec![
        ChildEnsure {
            type_name: ResourceTypeName::new("Process"),
            name: relay_process_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&relay_process_spec).map_err(|_| invalid())?,
            metadata: metadata.clone(),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Endpoint"),
            name: relay_endpoint_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&endpoint_spec).map_err(|_| invalid())?,
            metadata,
        },
    ])
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
        PROVIDER_REF, SECURITY_KEY_BINDING_RESOURCE_TYPE, SECURITY_KEY_REGISTRATIONS,
        SECURITY_KEY_SERVICE_RESOURCE_TYPE, SecurityKeyDriverArgs, ZoneId,
        security_key_descriptors,
    };

    fn descriptors() -> [d2b_resource_types::DriverDescriptor; 2] {
        security_key_descriptors(SecurityKeyDriverArgs {
            zone: ZoneId::parse("dev").expect("valid test zone"),
            controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
                .expect("generation"),
            facets: crate::test_support::recording_facets(Arc::new(
                crate::test_support::RecordingRuntime::default(),
            )),
        })
    }

    /// The declaration serves exactly the two converted security-key
    /// ResourceTypes, and the Service's relay is a declared creation.
    #[test]
    fn descriptors_declare_the_security_key_types() {
        let descriptors = descriptors();
        let types = descriptors
            .iter()
            .map(|descriptor| {
                descriptor
                    .resource_type
                    .to_resource_type_name()
                    .as_str()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                SECURITY_KEY_SERVICE_RESOURCE_TYPE.to_owned(),
                SECURITY_KEY_BINDING_RESOURCE_TYPE.to_owned()
            ]
        );
        assert!(descriptors[0].exportable, "a SecurityKey Service is exportable");
        assert!(!descriptors[1].exportable, "a SecurityKey Binding is not");
        let relay = descriptors[0]
            .creations
            .iter()
            .map(|creation| {
                (
                    creation.child.to_resource_type_name().as_str().to_owned(),
                    creation.provider_ref,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            relay,
            vec![
                ("Process".to_owned(), d2b_provider_process_minijail::PROVIDER_REF),
                ("Endpoint".to_owned(), PROVIDER_REF),
            ]
        );
    }

    /// The rows carry the preserved identities and resync cadence.
    #[test]
    fn rows_keep_the_preserved_identity() {
        assert_eq!(
            SECURITY_KEY_REGISTRATIONS[0].resource_type,
            SECURITY_KEY_SERVICE_RESOURCE_TYPE
        );
        assert_eq!(
            SECURITY_KEY_REGISTRATIONS[0].effect_id,
            "device-security-key-service"
        );
        assert_eq!(
            SECURITY_KEY_REGISTRATIONS[1].resource_type,
            SECURITY_KEY_BINDING_RESOURCE_TYPE
        );
        assert_eq!(
            SECURITY_KEY_REGISTRATIONS[1].effect_id,
            "device-security-key-binding"
        );
        for row in &SECURITY_KEY_REGISTRATIONS {
            assert_eq!(row.provider_ref, PROVIDER_REF);
            assert_eq!(row.resync, super::SECURITY_KEY_RESYNC);
        }
    }

    /// The relay Endpoint this driver declares carries a closed purpose
    /// token: `EndpointSpec. purpose` is a `BoundedToken`, so the dotted
    /// pre-wave spelling is an admission refusal.
    #[test]
    fn security_key_relay_endpoint_purpose_is_a_closed_token() {
        let owner = d2b_contracts_resource::v3::ResourceRef::parse(
            "security-key.d2bus.org.SecurityKeyService/key-a",
        )
        .expect("service ref");
        let device_ref =
            d2b_contracts_resource::v3::ResourceRef::parse("Device/key-a").expect("device ref");
        let relay_endpoint_ref =
            d2b_contracts_resource::v3::ResourceRef::parse("Endpoint/key-a-ctaphid-relay")
                .expect("endpoint ref");
        let device_uid = d2b_provider_toolkit::resource_uid(&[0x42; 16]).expect("device uid");
        let children = super::security_key_relay_child_ensures(
            &owner,
            &device_ref,
            &relay_endpoint_ref,
            &device_uid,
        )
        .expect("relay children");
        let endpoint = children
            .iter()
            .find(|child| child.type_name.as_str() == "Endpoint")
            .expect("relay Endpoint child");
        let value: serde_json::Value =
            serde_json::from_slice(&endpoint.spec).expect("endpoint spec json");
        let purpose = value
            .get("purpose")
            .and_then(serde_json::Value::as_str)
            .expect("relay endpoint purpose");
        assert_eq!(purpose, "security-key-ctaphid-relay");
        assert!(
            d2b_contracts_resource::v3::BoundedToken::parse(purpose).is_ok(),
            "relay endpoint purpose must be a closed BoundedToken"
        );
        serde_json::from_value::<d2b_provider_endpoint::endpoint::EndpointSpec>(value.clone())
            .expect("the relay Endpoint child decodes as the closed EndpointSpec");
    }
}
