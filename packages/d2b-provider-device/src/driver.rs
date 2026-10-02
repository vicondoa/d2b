//! The `Device` resource driver: the v3 `ResourceDriver` conversion of the
//! shared Runner's four Device Providers (tpm, usbip, security-key, gpu; R3,
//! R4, R30).
//!
//! The plane keys one driver per ResourceType, and `Device` is one
//! ResourceType served by four Providers whose rows are selected by the spec's
//! `providerRef`, so the type's driver lives here and declares all four rows.
//! The rows take their Provider identity from the realizer crates' exported
//! constants, so a Provider rename is a compile-time change instead of a
//! silent string edit.
//!
//! The Device rows realize nothing through manager resource rows of their own:
//! the Zone bundle declares the Device-owned worker rows
//! (`Process/swtpm-<device>`, `Process/gpu-<device>`, KTD13) and each family's
//! effect ensures its own controller-created rows (the TPM state Volume)
//! through the child surface. Diffing their owned rows against an empty
//! desired set would retire every declared row on the first reconcile pass, so
//! these components declare none and retire their whole owned subtree on
//! teardown instead.
//!
//! # Capability and authority
//!
//! A device grant is not a template name. Each family declares the closed set
//! of named capabilities it can deliver
//! ([`declared_device_functions`]) and the effect operation classes it admits
//! ([`device_effect_operations`]); the trusted inventory decides which of
//! those functions the host backs right now, and [`crate::binding`] is the
//! one source-side path that admits a consumer's `DeviceBindingRequest`
//! against those exact facts. Physical authority comes from the inventory's
//! opaque key, so a seccomp name, a launch role, or a device-node path can no
//! longer decide a device grant.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> the [`ProviderRow`] registrations under `Device`.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the spec decodes and
//!   names one of the four Providers.
//! - `observe` -> [`ResourceDriver::recover`]: the Provider-side realization
//!   is discovered in the effect, so the row adopts here.
//! - `plan`/`reconcile`/`execute_effect` -> [`ResourceDriver::reconcile`]
//!   behind [`DeviceDriverEffects`].
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`]: the family's finalizer semantics
//!   (TPM's stop-and-retain-volume, USBIP's supervisor finalize, the
//!   security-key relay retirement, GPU's authority release) run before the
//!   owned children retire.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ControllerGeneration, DeviceClass, DeviceEffectOperation, DeviceFunction, DeviceSpec,
    InventorySelector, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
use d2b_provider_toolkit::{
    ContextChildSurface, ProviderRow, SharedProviderDeclarationError, SharedProviderDriverArgs,
    SharedProviderDriverFactory, SharedProviderDriverStatus, SharedProviderEffectError,
    SharedProviderEffectOutcome, SharedProviderEffectRequest, SharedProviderFamily,
    SharedProviderFinalize, decode_metadata, owner_ref, resource_uid,
    shared_provider_spec_decoder,
};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
use serde_json::Value;

use crate::effects_service::DEVICE_EFFECTS_SERVICE;

/// The Device ResourceType served by the four hardware Providers.
pub const DEVICE_TYPE_NAME: &str = "Device";

/// Preserved self-resync for the Device Providers (old shared Runner repair
/// interval).
pub const DEVICE_RESYNC: Duration = Duration::from_secs(30);

/// The controller reference the TPM row's effects bind.
pub const TPM_CONTROLLER_REF: &str = "Process/device-tpm-controller";
/// The controller reference the USBIP Device row's effects bind.
pub const USBIP_CONTROLLER_REF: &str = "Process/device-usbip-controller";
/// The controller reference the security-key Device row's effects bind.
pub const SECURITY_KEY_CONTROLLER_REF: &str = "Process/device-security-key-controller";
/// The controller reference the GPU row's effects bind.
pub const GPU_CONTROLLER_REF: &str = "Process/device-gpu-controller";

/// The Device family's closed component vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceComponent {
    /// The TPM Provider's Device row (`Provider/device-tpm`).
    Tpm,
    /// The USBIP Provider's Device row (`Provider/device-usbip`).
    Usbip,
    /// The security-key Provider's Device row (`Provider/device-security-key`).
    SecurityKey,
    /// The GPU Provider's Device row (`Provider/device-gpu`).
    Gpu,
}

/// The rows this family declares, in the preserved registration order.
///
/// The Provider identities are the realizer crates' exported constants, so
/// the Device type's scope is declared by the crates that implement it.
pub const DEVICE_REGISTRATIONS: [ProviderRow<DeviceComponent>; 4] = [
    ProviderRow {
        resource_type: DEVICE_TYPE_NAME,
        component: DeviceComponent::Tpm,
        controller_ref: TPM_CONTROLLER_REF,
        provider_ref: d2b_provider_device_tpm::PROVIDER_REF,
        effect_id: "device-tpm",
        resync: DEVICE_RESYNC,
    },
    ProviderRow {
        resource_type: DEVICE_TYPE_NAME,
        component: DeviceComponent::Usbip,
        controller_ref: USBIP_CONTROLLER_REF,
        provider_ref: d2b_provider_device_usbip::PROVIDER_REF,
        effect_id: "device-usbip",
        resync: DEVICE_RESYNC,
    },
    ProviderRow {
        resource_type: DEVICE_TYPE_NAME,
        component: DeviceComponent::SecurityKey,
        controller_ref: SECURITY_KEY_CONTROLLER_REF,
        provider_ref: d2b_provider_device_security_key::PROVIDER_REF,
        effect_id: "device-security-key",
        resync: DEVICE_RESYNC,
    },
    ProviderRow {
        resource_type: DEVICE_TYPE_NAME,
        component: DeviceComponent::Gpu,
        controller_ref: GPU_CONTROLLER_REF,
        provider_ref: d2b_provider_device_gpu::PROVIDER_REF,
        effect_id: "device-gpu",
        resync: DEVICE_RESYNC,
    },
];

/// In-memory Provider state owned by one Device driver instance (one resource,
/// R6).
///
/// The old effects kept these in zone-wide maps keyed by resource uid; the
/// driver is already per resource, so the maps hold one slot each. The state
/// is never persisted: after a restart the Provider controllers rehydrate from
/// fresh evidence exactly as the old in-memory maps did.
#[derive(Default)]
pub struct DeviceResourceState {
    /// TPM child-resource controllers (old `tpm_controllers`).
    tpm_controllers: Arc<
        Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_tpm::TpmResourceController>>,
    >,
    /// GPU authority-fenced lifecycle controllers (old `gpu_controllers`).
    gpu_controllers:
        Arc<Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_gpu::GpuController>>>,
    /// GPU authority leases (old `gpu_authority_leases`). The GPU port's
    /// declared construction contract locks this cache with
    /// `parking_lot::Mutex`, so the driver-owned state uses the same lock.
    gpu_authority_leases: Arc<
        parking_lot::Mutex<
            std::collections::BTreeMap<[u8; 16], d2b_core_controller::authority::AuthorityLease>,
        >,
    >,
}

impl DeviceResourceState {
    /// The TPM child-resource controller cache (old `tpm_controllers`).
    ///
    /// Read-only access: callers lock the cache to take or store a
    /// controller; the cache itself cannot be replaced from outside the
    /// driver.
    pub fn tpm_controllers(
        &self,
    ) -> &Arc<
        Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_tpm::TpmResourceController>>,
    > {
        &self.tpm_controllers
    }

    /// The GPU authority-fenced lifecycle controller cache (old
    /// `gpu_controllers`).
    ///
    /// Read-only access: callers lock the cache to take or store a
    /// controller; the cache itself cannot be replaced from outside the
    /// driver.
    pub fn gpu_controllers(
        &self,
    ) -> &Arc<Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_gpu::GpuController>>>
    {
        &self.gpu_controllers
    }

    /// The GPU authority-lease cache (old `gpu_authority_leases`).
    ///
    /// The GPU port's declared construction contract locks this cache with
    /// `parking_lot::Mutex`, so the driver-owned state uses the same lock;
    /// the daemon hands the Arc clone to the GPU port unchanged.
    pub fn gpu_authority_leases(
        &self,
    ) -> &Arc<
        parking_lot::Mutex<
            std::collections::BTreeMap<[u8; 16], d2b_core_controller::authority::AuthorityLease>,
        >,
    > {
        &self.gpu_authority_leases
    }
}

/// The Provider effect surface the Device driver needs.
///
/// The production implementation owns the daemon-side Provider controllers,
/// the authority leases, and the broker dispatch; test doubles implement the
/// same seam. The driver-owned per-resource state travels back through
/// `state`, exactly as the old in-process controllers did.
#[async_trait]
pub trait DeviceDriverEffects: Send + Sync + 'static {
    /// Reconcile one Device row through its Provider's typed lifecycle.
    async fn reconcile_device(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one Device row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize_device(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

/// Everything the composition must construct to instantiate the Device driver
/// factory for one zone.
pub struct DeviceDriverArgs {
    /// The zone the driver serves.
    pub zone: ZoneId,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The daemon-supplied facet set the family's own effects
    /// implementation is built from (U12 device step): the driver never
    /// receives a daemon-built effect port (R2).
    pub facets: crate::facets::DeviceEffectFacets,
}

/// The family's declarations and typed Provider effect.
struct DeviceFamily {
    /// The Zone this family's rows live in.
    zone: ZoneId,
    effects: Arc<dyn DeviceDriverEffects>,
    /// The binding seam both halves share: the producing half reads the
    /// trusted inventory and the declared relationships through it, the
    /// serving half reads the same inventory plus the authority evidence it
    /// re-admits the committed row against before deciding presence. One value
    /// answers both, so the capability a source admits is the capability the
    /// row is decided against.
    bindings: Arc<dyn crate::binding::DeviceBindingEffects>,
}

impl DeviceFamily {
    /// Reconcile the `DeviceBinding` rows this committed `Device` row owns.
    ///
    /// This is the family's producing half, and it runs inside the reconcile
    /// verb the row is already driven by rather than in a pass of its own.
    /// Its commit and its retirement are both scoped to the one type this
    /// source owns relationships in, so the Zone-declared worker rows and the
    /// effect-created rows the trait's `desired_children` hook declines to
    /// diff are never touched here.
    async fn reconcile_binding_children(
        &self,
        ctx: &mut ResourceContext,
        component: DeviceComponent,
        spec: &Value,
    ) -> Result<(), SharedProviderDeclarationError> {
        let target = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
        let metadata = decode_metadata(ctx.metadata())
            .map_err(|_| SharedProviderDeclarationError::SpecInvalid)?;
        let status = ctx
            .status::<SharedProviderDriverStatus>()
            .and_then(|status| status.resource.clone());
        let owned = ctx
            .children()
            .await
            .map_err(|_| SharedProviderDeclarationError::ChildMutation)?;

        // The child surface borrows the context for the pass, and the owned
        // set is already read, so nothing else needs the context while it is
        // held.
        let surface = ContextChildSurface::new(ctx);
        let request = SharedProviderEffectRequest {
            zone: self.zone.clone(),
            operation_id: crate::binding::binding_operation_id(target.name.as_str()),
            target,
            uid,
            generation,
            spec,
            metadata,
            status,
            children: &surface,
        };
        // The trusted inventory is an observation: a Zone that cannot resolve
        // one admits nothing and retires nothing, so the pass fails retryably
        // rather than reading an absent observation as an absence.
        let inventory = self
            .bindings
            .device_inventory(&request)
            .await
            .map_err(|_| SharedProviderDeclarationError::ChildMutation)?;
        let declared = self.bindings.declared_bindings(&request).await;
        let production = crate::binding::produce_binding_rows(
            &request,
            component,
            &inventory,
            &declared,
            &owned,
        )
        .await
        .map_err(|error| match error {
            crate::binding::BindingProductionError::ProviderRefused => {
                SharedProviderDeclarationError::SpecInvalid
            }
            crate::binding::BindingProductionError::ChildMutation => {
                SharedProviderDeclarationError::ChildMutation
            }
        })?;
        if let Some(refusal) = production.refusal() {
            // Named, not approximated: a source that cannot show its
            // admission evidence commits nothing and says which fact was
            // missing, so the gap is visible instead of silently producing an
            // empty desired set.
            tracing::warn!(
                device = %request.target,
                refusal = %refusal,
                "the Device source committed no device binding row",
            );
        }
        Ok(())
    }
}

#[async_trait]
impl SharedProviderFamily for DeviceFamily {
    type Component = DeviceComponent;
    type State = DeviceResourceState;

    fn rows(&self) -> &'static [ProviderRow<Self::Component>] {
        &DEVICE_REGISTRATIONS
    }

    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        component: DeviceComponent,
        spec: &Value,
    ) -> Result<Option<Vec<d2b_resource_runtime::context::ChildEnsure>>, SharedProviderDeclarationError>
    {
        // The Device-owned worker rows belong to the Zone bundle and the
        // controller-created rows to the family effect; see the module header.
        // Diffing those against any desired set would retire them, so this
        // hook keeps declining the shared diff.
        //
        // The `DeviceBinding` rows this source DOES own are reconciled here
        // instead, scoped to that one type: the producing pass commits and
        // retires through the manager's child surface and touches nothing
        // else the row owns.
        self.reconcile_binding_children(ctx, component, spec).await?;
        Ok(None)
    }

    fn declared_dependency_refs(
        &self,
        component: DeviceComponent,
        spec: &Value,
        metadata: &Value,
    ) -> Vec<ResourceRef> {
        declared_dependency_refs(component, spec, metadata)
    }

    async fn effect(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
        self.effects.reconcile_device(component, request, state).await
    }

    async fn finalize(
        &self,
        component: DeviceComponent,
        request: &SharedProviderEffectRequest<'_>,
        state: &DeviceResourceState,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
        self.effects.finalize_device(component, request, state).await
    }
}

/// The dependency references one Device row declares.
///
/// The TPM and GPU Devices are owned by the Guest whose worker rows they gate,
/// so their owner row is the dependency the effects read. The USBIP and
/// security-key Device rows read their own Services through the effect.
pub fn declared_dependency_refs(
    component: DeviceComponent,
    _spec: &Value,
    metadata: &Value,
) -> Vec<ResourceRef> {
    match component {
        DeviceComponent::Tpm | DeviceComponent::Gpu => {
            owner_ref(metadata).ok().into_iter().collect()
        }
        DeviceComponent::Usbip | DeviceComponent::SecurityKey => Vec::new(),
    }
}

/// Resolve the family one committed Device row names.
///
/// The `Device` type is served by four Providers and one driver keys them
/// all, so a row's own `providerRef` is the only thing that selects the
/// family. This is the one spelling of that selection: a Provider outside
/// the four is `None`, and no other field of the row can move a capability or
/// a storage grant between families.
pub fn component_for_provider(provider_ref: &str) -> Option<DeviceComponent> {
    match provider_ref {
        d2b_provider_device_tpm::PROVIDER_REF => Some(DeviceComponent::Tpm),
        d2b_provider_device_usbip::PROVIDER_REF => Some(DeviceComponent::Usbip),
        d2b_provider_device_security_key::PROVIDER_REF => Some(DeviceComponent::SecurityKey),
        d2b_provider_device_gpu::PROVIDER_REF => Some(DeviceComponent::Gpu),
        _ => None,
    }
}

/// The execution domains the Device type can be reconciled in.
const DEVICE_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The named capabilities each Device family can ever admit, for the bus
/// class its row declares.
///
/// This is the provider-owned replacement for a template table that named
/// device nodes: a family declares the closed set of functions its
/// realization knows how to deliver, and the trusted inventory decides which
/// of them the host actually backs right now. A request may name one of
/// these names and nothing else, so a device grant can no longer be reached
/// by spelling a template.
///
/// The GPU vocabulary carries the NVIDIA nodes as ordinary named
/// capabilities. They are not a wider default: a consumer that needs one
/// must hold an admitted binding for it, so a decode mode selects which
/// admitted capability the worker reaches rather than which node path the
/// launch is handed.
///
/// An emulated Device declares no inventory selector at all - the closed
/// `InventorySelector` union says so outright ("an emulated device carries
/// no selector") - and its capability is the Provider's own emulation rather
/// than a host device node. Falling through to the empty arm for that shape
/// produced an empty inventory, and an empty inventory is a refusal, so the
/// row was abandoned before its Provider controller ran: the TPM controller
/// never committed the state Volume its long-lived worker opens by pathname,
/// the directory never landed, and the worker's spawn was then refused for
/// an absent state-directory leaf. The emulated TPM therefore names the same
/// `tpm` capability a selected physical TPM names. A *physical* Device that
/// declares no selector is still refused: that is a malformed declaration,
/// not an emulated one.
pub fn declared_device_functions(
    component: DeviceComponent,
    spec: &DeviceSpec,
) -> Vec<DeviceFunction> {
    let selector = spec.inventory().selector();
    let functions: &[&str] = match (component, selector) {
        (DeviceComponent::Tpm, None) if spec.device_class() == DeviceClass::Emulated => &["tpm"],
        (DeviceComponent::Tpm, Some(InventorySelector::Tpm { .. })) => &["tpm"],
        (DeviceComponent::Gpu, Some(InventorySelector::Drm { .. })) | (
            DeviceComponent::Gpu,
            Some(InventorySelector::Pci { .. }),
        ) => &[
            "dri",
            "render-node",
            "udmabuf",
            "nvidia-ctl",
            "nvidia-uvm",
            "nvidia-device",
        ],
        (DeviceComponent::Usbip, Some(InventorySelector::Usb { .. })) => &["usb"],
        (
            DeviceComponent::SecurityKey,
            Some(InventorySelector::Hidraw { .. }),
        ) => &["hidraw"],
        _ => &[],
    };
    functions
        .iter()
        .filter_map(|name| DeviceFunction::parse(*name).ok())
        .collect()
}

/// The effect operation classes each Device family admits.
///
/// A relationship's operations are the source's own declaration, and a
/// helper leg may only drive a subset of them, so widening one family's
/// reachable effects is a change to this table rather than to a launch
/// argument.
pub const fn device_effect_operations(component: DeviceComponent) -> &'static [DeviceEffectOperation] {
    match component {
        DeviceComponent::Tpm => &[DeviceEffectOperation::PrepareStateDir, DeviceEffectOperation::SpawnRunner],
        DeviceComponent::Gpu => &[DeviceEffectOperation::OpenDevice, DeviceEffectOperation::SpawnRunner],
        DeviceComponent::Usbip => &[
            DeviceEffectOperation::SpawnRunner,
            DeviceEffectOperation::ApplyNftablesProjection,
        ],
        DeviceComponent::SecurityKey => &[
            DeviceEffectOperation::SecurityKeyOpenDevice,
            DeviceEffectOperation::SecurityKeyApplyUdevRules,
        ],
    }
}

/// The resource types the Device realizations read while reconciling: the
/// owning Guest, the Host execution target, the declared worker rows, the
/// state Volume, the admitted device bindings the source arbitrates, the
/// relay/worker Endpoints, and the Services the USBIP and security-key
/// Device rows are admitted by.
const DEVICE_READS: &[WellKnownType] = &[
    WellKnownType::GUEST,
    WellKnownType::HOST,
    WellKnownType::VOLUME,
    WellKnownType::VOLUME_BINDING,
    WellKnownType::PROCESS,
    WellKnownType::ENDPOINT,
    WellKnownType::USB_SERVICE,
    WellKnownType::SECURITY_KEY_SERVICE,
];

/// The Device type's driver declaration.
///
/// `Device` is `BUILTIN | STARTUP | RUNTIME` (the RUNTIME bit is present):
/// hardware presence is host-dependent, so the driver may arrive late. The
/// type is not exportable, and the Device rows' worker rows are declared by
/// the Zone bundle rather than here.
///
/// The `DeviceBinding` rows the reconcile pass mints are declared as
/// creations only once the child type names the Provider that serves it. It
/// does not: `DeviceBinding` is served by this same crate's binding driver,
/// which selects no Provider of its own, so there is no `(child, provider)`
/// pair to declare. The rows are named from the relationship key and owned by
/// the `Device` row, and the producing pass only ever adds or removes that
/// one type.
pub fn device_descriptor(args: DeviceDriverArgs) -> DriverDescriptor {
    // One value answers both seams the family holds: the driver's typed
    // Provider effect, and the binding halves' own read of the trusted
    // inventory and the declared relationships. Building it from the
    // daemon-supplied facets keeps the construction site free of any
    // externally built port (R2).
    let effects = Arc::new(crate::effects_service::DeviceEffects::new(args.facets));
    DriverDescriptor {
        resource_type: WellKnownType::DEVICE,
        allowed_sources: AllowedSources::BUILTIN
            | AllowedSources::STARTUP
            | AllowedSources::RUNTIME,
        verbs: CONVERTED_TYPE_VERBS,
        execution: DEVICE_EXECUTION_DOMAINS,
        exportable: false,
        reads: DEVICE_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[DEVICE_EFFECTS_SERVICE],
        decoder: shared_provider_spec_decoder(),
        factory: Arc::new(SharedProviderDriverFactory::new(
            SharedProviderDriverArgs {
                zone: args.zone.clone(),
                controller_generation: args.controller_generation,
                family: Arc::new(DeviceFamily {
                    zone: args.zone,
                    effects: effects.clone(),
                    bindings: effects,
                }),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceComponent, declared_device_functions};
    use d2b_contracts_resource::v3::DeviceSpec;

    fn spec(value: serde_json::Value) -> DeviceSpec {
        serde_json::from_value(value).expect("the wire spec decodes")
    }

    fn emulated_tpm(selector: serde_json::Value) -> DeviceSpec {
        spec(serde_json::json!({
            "deviceClass": "emulated",
            "arbitration": "exclusive",
            "maxConcurrentClaims": 1,
            "inventory": { "selector": selector }
        }))
    }

    fn physical(selector: serde_json::Value) -> serde_json::Result<DeviceSpec> {
        serde_json::from_value(serde_json::json!({
            "deviceClass": "physical",
            "arbitration": "exclusive",
            "maxConcurrentClaims": 1,
            "inventory": { "selector": selector }
        }))
    }

    /// The emulated TPM a Zone declares carries no host device node, so it
    /// declares no selector at all. It still resolves its one named
    /// capability: an empty inventory is a refusal, and a refusal here would
    /// abandon the row before its Provider controller runs, which is what
    /// left the TPM state Volume uncommitted and its worker's spawn refused
    /// for an absent state-directory leaf.
    #[test]
    fn an_emulated_tpm_declares_no_selector_and_still_names_its_capability() {
        let functions = declared_device_functions(DeviceComponent::Tpm, &emulated_tpm(
            serde_json::Value::Null,
        ));
        assert_eq!(
            functions.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            vec!["tpm"],
            "the emulated TPM names the capability its worker grant is minted from"
        );
    }

    /// A physical TPM names a stable operator label and keeps the same
    /// capability name, so the two shapes agree on the vocabulary.
    #[test]
    fn a_selected_physical_tpm_names_the_same_capability() {
        let spec = spec(serde_json::json!({
            "deviceClass": "physical",
            "arbitration": "exclusive",
            "maxConcurrentClaims": 1,
            "inventory": { "selector": { "busClass": "tpm", "label": "tpm0" } }
        }));
        let functions = declared_device_functions(DeviceComponent::Tpm, &spec);
        assert_eq!(
            functions.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            vec!["tpm"]
        );
    }

    /// The closed `InventorySelector` union is discriminated on `busClass`,
    /// so an *empty object* is not a member of it. A fixture that spells an
    /// emulated Device's absent selector that way - Nix's empty attribute set
    /// serializes to `{}` - produces a spec that does not decode at all, and
    /// the resulting failure abandons the `Device` row before its Provider
    /// controller runs. This pins the refusal so the spelling cannot come
    /// back.
    #[test]
    fn an_empty_selector_object_is_not_a_declared_selector() {
        let error = physical(serde_json::json!({})).expect_err("an empty selector is refused");
        assert!(
            error.to_string().contains("busClass"),
            "the closed union refuses a selector that names no bus class: {error}"
        );
    }

    /// A physical Device always names a stable operator-defined label, so a
    /// physical GPU with no selector never reaches the capability table at
    /// all. This is the fail-closed half of the fix: the emulated TPM is
    /// admitted because its class says it carries no node, not because the
    /// table stopped refusing missing selectors.
    #[test]
    fn a_physical_device_with_no_selector_is_refused_by_the_contract() {
        physical(serde_json::Value::Null).expect_err("a physical Device must name a selector");
    }

    /// A GPU Device that names no DRM or PCI selector resolves nothing, so an
    /// unbacked GPU grant stays unreachable.
    #[test]
    fn a_gpu_selector_that_names_no_backed_node_resolves_nothing() {
        let spec = spec(serde_json::json!({
            "deviceClass": "physical",
            "arbitration": "exclusive",
            "maxConcurrentClaims": 1,
            "inventory": { "selector": { "busClass": "usb", "label": "not-a-gpu" } }
        }));
        assert!(declared_device_functions(DeviceComponent::Gpu, &spec).is_empty());
    }
}
