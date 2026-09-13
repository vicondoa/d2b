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
use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid};
use d2b_provider_toolkit::{
    ProviderRow, SharedProviderDeclarationError, SharedProviderDriverArgs,
    SharedProviderDriverFactory, SharedProviderEffectError, SharedProviderEffectOutcome,
    SharedProviderEffectRequest, SharedProviderFamily, SharedProviderFinalize, owner_ref,
    shared_provider_spec_decoder,
};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};
use serde_json::Value;

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
    pub tpm_controllers: Arc<
        Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_tpm::TpmResourceController>>,
    >,
    /// GPU authority-fenced lifecycle controllers (old `gpu_controllers`).
    pub gpu_controllers:
        Arc<Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_gpu::GpuController>>>,
    /// GPU authority leases (old `gpu_authority_leases`).
    pub gpu_authority_leases: Arc<
        Mutex<std::collections::BTreeMap<[u8; 16], d2b_core_controller::authority::AuthorityLease>>,
    >,
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
    pub zone: String,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The daemon-realized effect port the driver drives.
    pub effects: Arc<dyn DeviceDriverEffects>,
}

/// The family's declarations and typed Provider effect.
struct DeviceFamily {
    effects: Arc<dyn DeviceDriverEffects>,
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
        _ctx: &mut ResourceContext,
        _component: DeviceComponent,
        _spec: &Value,
    ) -> Result<Option<Vec<d2b_resource_runtime::context::ChildEnsure>>, SharedProviderDeclarationError>
    {
        // The Device-owned worker rows belong to the Zone bundle and the
        // controller-created rows to the family effect; see the module header.
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

/// The resource verbs the Device type supports.
const DEVICE_VERBS: &[&str] = &[
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

/// The execution domains the Device type can be reconciled in.
const DEVICE_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The resource types the Device realizations read while reconciling: the
/// owning Guest, the Host execution target, the declared worker rows, the
/// state Volume, the relay/worker Endpoints, and the Services the USBIP and
/// security-key Device rows are admitted by.
const DEVICE_READS: &[WellKnownType] = &[
    WellKnownType::GUEST,
    WellKnownType::HOST,
    WellKnownType::VOLUME,
    WellKnownType::PROCESS,
    WellKnownType::ENDPOINT,
    WellKnownType::USB_SERVICE,
    WellKnownType::SECURITY_KEY_SERVICE,
];

/// The Device type's driver declaration.
///
/// `Device` is `BUILTIN | STARTUP | RUNTIME` (the RUNTIME bit is present):
/// hardware presence is host-dependent, so the driver may arrive late. The
/// type is not exportable, and the Device rows create no children through
/// this declaration - their worker rows are declared by the Zone bundle.
pub fn device_descriptor(args: DeviceDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::DEVICE,
        allowed_sources: AllowedSources::BUILTIN
            | AllowedSources::STARTUP
            | AllowedSources::RUNTIME,
        verbs: DEVICE_VERBS,
        execution: DEVICE_EXECUTION_DOMAINS,
        exportable: false,
        reads: DEVICE_READS,
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: shared_provider_spec_decoder(),
        factory: Arc::new(SharedProviderDriverFactory::new(
            SharedProviderDriverArgs {
                zone: args.zone,
                controller_generation: args.controller_generation,
                family: Arc::new(DeviceFamily {
                    effects: args.effects,
                }),
            },
        )),
    }
}
