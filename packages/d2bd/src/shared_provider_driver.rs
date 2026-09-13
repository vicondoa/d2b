//! Shared host-provider family drivers (U12 wave 2): the v3 `ResourceDriver`
//! conversion of the U8 shared Runner family - Network and the Device
//! Providers (tpm, usbip, security-key, gpu; R3, R4, R30).
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`SharedProviderDriverFactory`] registration under the
//!   family's ResourceTypes.
//! - `validate_spec` -> [`ResourceDriver::validate`]: the spec decodes and
//!   names a Provider this factory owns for the row's ResourceType.
//! - `observe` -> [`ResourceDriver::recover`]: owned-child adoption.
//! - finalizer enrollment + `plan`/`reconcile`/`execute_effect` ->
//!   [`ResourceDriver::reconcile`]: the desired child set is ensured through
//!   the manager child API (committed before the child actor exists, F1),
//!   owned children the desired set no longer derives are retired in the
//!   family's preserved order, the typed Provider effect runs behind
//!   [`SharedProviderDriverEffects`], and the in-memory status projection is
//!   published with `ctx.set_status` (R11) plus a self-`requeue_after` while
//!   the family is not converged.
//! - `prepare_finalize`/`execute_finalize`/`finalize` ->
//!   [`ResourceDriver::delete`]: the family's preserved teardown ordering and
//!   the per-Provider finalizer semantics (Network's staged fabric finalizer,
//!   TPM's stop-and-retain-volume, USBIP's supervisor finalize, the
//!   SecurityKey relay retirement, GPU's authority release) run before the
//!   owned children retire.
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11); the old
//!   durable status and its sanitizers are gone with it.
//!
//! Process work never happens here (KTD13): the Provider effect ports own
//! the child-row ensures and the phase gates, and any broker work they still
//! need is a state operation, not a launch. Child Process and Endpoint
//! resources are ensured as manager rows and launched by the Process/Endpoint
//! drivers, which is why the Device worker effects read the declared
//! `Process/swtpm-<device>` / `Process/gpu-<device>` rows instead of spawning
//! them.
//!
//! Cross-resource readiness (a child's or dependency's live phase) is
//! observable from an effect through [`SharedProviderChildSurface::view`],
//! which reads the manager plane's live view for a row of the driving
//! resource; the driver itself only registers
//! `ctx.watch(.., WatchCondition::Ready)` edges so dependency and child
//! readiness wake it, and it never fabricates a readiness it cannot observe.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
    execution_policy::ExecutionPolicy, guest::GuestSpec, network::NetworkSpec,
};
use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildIntent;
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::spec_store::EnsureOutcome;
use serde_json::{Value, json};

/// The Network ResourceType served by the network-local Provider.
pub(crate) const NETWORK_TYPE_NAME: &str = "Network";
/// The Device ResourceType shared by the tpm/usbip/security-key/gpu Providers.
pub(crate) const DEVICE_TYPE_NAME: &str = "Device";

/// Canonical Host execution target (the old shared Runner's Host ref).
pub(crate) const HOST_REF: &str = "Host/host-system";

/// Preserved reconcile self-resync for rows whose Provider is not converged
/// (old shared Runner repair interval for the Network Provider).
pub(crate) const NETWORK_RESYNC: Duration = Duration::from_secs(30);
/// Preserved self-resync for the Device Providers.
pub(crate) const DEVICE_RESYNC: Duration = Duration::from_secs(30);

/// One ResourceType/Provider row of the U8 shared Runner family.
///
/// The table pins the provider identity every effect call binds: the
/// controller reference, the Provider reference, and the repair cadence the
/// old descriptor carried. It replaces the old runner registration rows; the
/// descriptors themselves are gone with the old Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SharedProviderRegistration {
    pub(crate) kind: SharedProviderKind,
    pub(crate) controller_ref: &'static str,
    pub(crate) provider_ref: &'static str,
    pub(crate) resource_type: &'static str,
    /// Preserved resync cadence (old `repair_interval_secs`).
    pub(crate) resync: Duration,
}

/// The nine U8 shared Runner registrations, in the preserved order.
pub(crate) const SHARED_PROVIDER_REGISTRATIONS: [SharedProviderRegistration; 9] = [
    SharedProviderRegistration {
        kind: SharedProviderKind::Network,
        controller_ref: "Process/network-local-controller",
        provider_ref: "Provider/network-local",
        resource_type: NETWORK_TYPE_NAME,
        resync: NETWORK_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::TpmDevice,
        controller_ref: "Process/device-tpm-controller",
        provider_ref: "Provider/device-tpm",
        resource_type: DEVICE_TYPE_NAME,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::UsbipDevice,
        controller_ref: "Process/device-usbip-controller",
        provider_ref: "Provider/device-usbip",
        resource_type: DEVICE_TYPE_NAME,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::UsbipService,
        controller_ref: "Process/device-usbip-service-controller",
        provider_ref: "Provider/device-usbip",
        resource_type: d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::UsbipBinding,
        controller_ref: "Process/device-usbip-binding-controller",
        provider_ref: "Provider/device-usbip",
        resource_type: d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::SecurityKeyDevice,
        controller_ref: "Process/device-security-key-controller",
        provider_ref: "Provider/device-security-key",
        resource_type: DEVICE_TYPE_NAME,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::SecurityKeyService,
        controller_ref: "Process/device-security-key-service-controller",
        provider_ref: "Provider/device-security-key",
        resource_type: d2b_provider_device_security_key::SECURITY_KEY_SERVICE_RESOURCE_TYPE,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::SecurityKeyBinding,
        controller_ref: "Process/device-security-key-binding-controller",
        provider_ref: "Provider/device-security-key",
        resource_type: d2b_provider_device_security_key::SECURITY_KEY_BINDING_RESOURCE_TYPE,
        resync: DEVICE_RESYNC,
    },
    SharedProviderRegistration {
        kind: SharedProviderKind::GpuDevice,
        controller_ref: "Process/device-gpu-controller",
        provider_ref: "Provider/device-gpu",
        resource_type: DEVICE_TYPE_NAME,
        resync: DEVICE_RESYNC,
    },
];

/// Closed Provider handler set served by this family (old
/// `SharedProviderResourceKind`, U8 rows only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SharedProviderKind {
    Network,
    TpmDevice,
    UsbipDevice,
    UsbipService,
    UsbipBinding,
    SecurityKeyDevice,
    SecurityKeyService,
    SecurityKeyBinding,
    GpuDevice,
}

/// USBIP resource owner selected by a registration row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UsbipComponent {
    Device,
    Service,
    Binding,
}

/// SecurityKey resource owner selected by a registration row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecurityKeyComponent {
    Device,
    Service,
    Binding,
}

impl SharedProviderKind {
    /// Resolve the family row for one (ResourceType, Provider) pair.
    ///
    /// The old Runner selected the row by its registration tuple; the new
    /// plane knows only the key, so the row's Provider reference - carried by
    /// the spec and fenced by `validate` - selects the Provider handler. A
    /// Provider this factory does not own is refused, never guessed.
    pub(crate) fn from_type_and_provider(
        resource_type: &str,
        provider_ref: Option<&str>,
    ) -> Result<Self, SharedProviderEffectError> {
        let Some(provider_ref) = provider_ref else {
            return Err(SharedProviderEffectError::InvalidResource);
        };
        SHARED_PROVIDER_REGISTRATIONS
            .iter()
            .find(|registration| {
                registration.resource_type == resource_type
                    && registration.provider_ref == provider_ref
            })
            .map(|registration| registration.kind)
            .ok_or(SharedProviderEffectError::InvalidResource)
    }

    pub(crate) const fn registration(self) -> SharedProviderRegistration {
        SHARED_PROVIDER_REGISTRATIONS[self.index()]
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Network => 0,
            Self::TpmDevice => 1,
            Self::UsbipDevice => 2,
            Self::UsbipService => 3,
            Self::UsbipBinding => 4,
            Self::SecurityKeyDevice => 5,
            Self::SecurityKeyService => 6,
            Self::SecurityKeyBinding => 7,
            Self::GpuDevice => 8,
        }
    }

    pub(crate) const fn effect_id(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::TpmDevice => "device-tpm",
            Self::UsbipDevice => "device-usbip",
            Self::UsbipService => "device-usbip-service",
            Self::UsbipBinding => "device-usbip-binding",
            Self::SecurityKeyDevice => "device-security-key",
            Self::SecurityKeyService => "device-security-key-service",
            Self::SecurityKeyBinding => "device-security-key-binding",
            Self::GpuDevice => "device-gpu",
        }
    }

    pub(crate) const fn provider_ref(self) -> &'static str {
        self.registration().provider_ref
    }

    pub(crate) const fn controller_ref(self) -> &'static str {
        self.registration().controller_ref
    }

    /// The family row's ResourceType. The registration-table test reads it as
    /// the table's own consistency check; the daemon selects a row through
    /// the ResourceType it is already driving, so no production caller reads
    /// it back.
    #[allow(dead_code)]
    pub(crate) const fn resource_type(self) -> &'static str {
        self.registration().resource_type
    }

    pub(crate) const fn resync(self) -> Duration {
        self.registration().resync
    }

    pub(crate) const fn usbip_component(self) -> Option<UsbipComponent> {
        match self {
            Self::UsbipDevice => Some(UsbipComponent::Device),
            Self::UsbipService => Some(UsbipComponent::Service),
            Self::UsbipBinding => Some(UsbipComponent::Binding),
            _ => None,
        }
    }

    pub(crate) const fn security_key_component(self) -> Option<SecurityKeyComponent> {
        match self {
            Self::SecurityKeyDevice => Some(SecurityKeyComponent::Device),
            Self::SecurityKeyService => Some(SecurityKeyComponent::Service),
            Self::SecurityKeyBinding => Some(SecurityKeyComponent::Binding),
            _ => None,
        }
    }
}

/// Every ResourceType this factory serves (KTD4 Phase A partition).
pub(crate) const SHARED_PROVIDER_TYPES: [&str; 6] = [
    NETWORK_TYPE_NAME,
    DEVICE_TYPE_NAME,
    d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE,
    d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE,
    d2b_provider_device_security_key::SECURITY_KEY_SERVICE_RESOURCE_TYPE,
    d2b_provider_device_security_key::SECURITY_KEY_BINDING_RESOURCE_TYPE,
];

// ---------------------------------------------------------------------------
// Typed Provider effect boundary (old `SharedProviderEffectExecutor` rows)
// ---------------------------------------------------------------------------

/// Result returned by one typed Provider effect adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderEffectPhase {
    Ready,
    Pending,
}

/// One Provider effect outcome: the phase the old effect returned plus the
/// `status.resource` projection the old status candidate published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedProviderEffectOutcome {
    pub(crate) phase: SharedProviderEffectPhase,
    pub(crate) resource_projection: Option<Value>,
}

impl SharedProviderEffectOutcome {
    pub(crate) const fn phase(phase: SharedProviderEffectPhase) -> Self {
        Self {
            phase,
            resource_projection: None,
        }
    }

    pub(crate) fn projection(
        phase: SharedProviderEffectPhase,
        resource_projection: Value,
    ) -> Self {
        Self {
            phase,
            resource_projection: Some(resource_projection),
        }
    }
}

/// Outcome of one Provider teardown stage (old `execute_finalize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderFinalize {
    /// Cleanup finished; the owned children may retire.
    Complete,
    /// Cleanup is progressing; the owner is re-entered (old `Pending`).
    Pending,
}

/// Closed failure surface for shared Provider adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedProviderEffectError {
    /// Cleanup is progressing and the owner should be re-entered.
    ///
    /// The converted Device Providers report teardown progress through
    /// [`SharedProviderFinalize::Pending`] instead, so no port constructs
    /// this variant today; the effect-error arm that maps it onto
    /// [`SharedProviderDriverErrorKind::FinalizePending`] stays closed for the
    /// ports still being converted (U17 Device family).
    #[allow(dead_code)]
    Pending,
    /// The Provider path is not currently available and should retry.
    Unavailable,
    /// Fresh resource or assignment evidence failed closed.
    InvalidResource,
}

impl core::fmt::Display for SharedProviderEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Pending => "shared-provider-effect-pending",
            Self::Unavailable => "shared-provider-effect-unavailable",
            Self::InvalidResource => "shared-provider-resource-invalid",
        })
    }
}

impl std::error::Error for SharedProviderEffectError {}

// ---------------------------------------------------------------------------
// Driver error classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedProviderDriverErrorKind {
    /// The durable spec did not decode or names a Provider outside the row's
    /// ResourceType: terminal, retrying cannot change the stored spec.
    SpecInvalid,
    /// A manager child mutation failed (retryable: the manager owns retries).
    ChildMutation,
    /// The provider path is temporarily unavailable.
    ProviderUnavailable,
    /// A Provider teardown stage is still progressing.
    FinalizePending,
}

impl SharedProviderDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid => FailureClass::Terminal,
            Self::ChildMutation | Self::ProviderUnavailable | Self::FinalizePending => {
                FailureClass::Retryable
            }
        }
    }

    const fn code(self) -> &'static str {
        match self {
            Self::SpecInvalid => "shared-provider-spec-invalid",
            Self::ChildMutation => "shared-provider-child-mutation",
            Self::ProviderUnavailable => "shared-provider-unavailable",
            Self::FinalizePending => "shared-provider-finalize-pending",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SharedProviderDriverError {
    kind: SharedProviderDriverErrorKind,
    op: DriverOp,
}

impl SharedProviderDriverError {
    const fn new(kind: SharedProviderDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for SharedProviderDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for SharedProviderDriverError {}

/// Typed in-memory status projection (R11: never persisted). Carries the
/// closed phase the old status candidate published plus the Provider's
/// `status.resource` projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedProviderDriverStatus {
    pub(crate) phase: SharedProviderEffectPhase,
    pub(crate) resource: Option<Value>,
}

impl SharedProviderDriverStatus {
    /// The closed phase string this status was built from. No daemon read
    /// path maps the typed status back to a phase today (the runtime phase
    /// comes from `ResourceStatus`); the projection readers the U17 family
    /// status work adds read it.
    #[allow(dead_code)]
    pub(crate) const fn phase(&self) -> &'static str {
        match self.phase {
            SharedProviderEffectPhase::Ready => "Ready",
            SharedProviderEffectPhase::Pending => "Pending",
        }
    }
}

// ---------------------------------------------------------------------------
// Spec decode (manager-wired)
// ---------------------------------------------------------------------------

/// Decoded shared-provider spec envelope: the exact stored spec bytes and the
/// canonical spec document the family's Provider handlers read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedProviderSpecEnvelope {
    /// The exact stored spec bytes; never rewritten by this driver.
    raw: Vec<u8>,
    value: Value,
}

impl SharedProviderSpecEnvelope {
    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    /// The exact stored spec bytes. The family's handlers read [`Self::value`]
    /// and the driver never rewrites the stored envelope, so nothing reads the
    /// raw bytes back today; the audit-facing readers the U9/U10 cutover sized
    /// this envelope for own it.
    #[allow(dead_code)]
    pub(crate) fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The spec's Provider reference (the family row selector).
    pub(crate) fn provider_ref(&self) -> Option<&str> {
        self.value.get("providerRef").and_then(Value::as_str)
    }
}

/// Closed decode error for a shared-provider spec envelope.
#[derive(Debug, thiserror::Error)]
#[error("shared provider spec must be a JSON object")]
pub(crate) struct SharedProviderSpecDecodeError;

/// The manager-wired decode hook for the family's ResourceTypes.
pub(crate) fn shared_provider_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        let value = serde_json::from_slice::<Value>(bytes)
            .map_err(|_| SharedProviderSpecDecodeError)?;
        if !value.is_object() {
            return Err(SharedProviderSpecDecodeError);
        }
        Ok(SharedProviderSpecEnvelope {
            raw: bytes.to_vec(),
            value,
        })
    })
}

/// Decode one row's metadata envelope (empty metadata is the empty object).
pub(crate) fn decode_metadata(raw: &[u8]) -> Result<Value, SharedProviderEffectError> {
    if raw.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice::<Value>(raw).map_err(|_| SharedProviderEffectError::InvalidResource)
}

/// The owner reference the old effects read from `/metadata/ownerRef`.
pub(crate) fn owner_ref(metadata: &Value) -> Result<ResourceRef, SharedProviderEffectError> {
    metadata
        .get("ownerRef")
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or(SharedProviderEffectError::InvalidResource)
}

/// Convert one durable 16-byte uid to its canonical identity (the manager
/// persists the uid as bytes; the Provider effects key on the canonical
/// string).
pub(crate) fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, SharedProviderEffectError> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| SharedProviderEffectError::InvalidResource)
}

// ---------------------------------------------------------------------------
// Manager-routed child surface
// ---------------------------------------------------------------------------

/// Manager-routed child mutations handed to one Provider effect call.
///
/// The driver owns this surface (it is the only holder of the resource's
/// [`ResourceContext`]); the Provider effects use it to ensure and delete the
/// child resources they declare. Every ensure rides
/// [`ResourceContext::ensure_child`], so the manager commits the child row
/// BEFORE the child actor exists (F1, AE1).
#[async_trait]
pub(crate) trait SharedProviderChildSurface: Send + Sync {
    /// Create or update one child row through the manager.
    async fn ensure(&self, child: ChildEnsure) -> Result<EnsureOutcome, SharedProviderEffectError>;
    /// Delete one owned child row through the manager (idempotent).
    async fn delete(&self, key: &ResourceKey) -> Result<(), SharedProviderEffectError>;
    /// The live manager view of one child row (absent when no row exists).
    ///
    /// The row-owned launch path (U17) gates on this: a Provider effect
    /// ensures the declared Process/EphemeralProcess/Endpoint child and reads
    /// its published phase here instead of holding a raw broker handle.
    async fn view(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, SharedProviderEffectError>;
}

/// [`SharedProviderChildSurface`] over the calling resource's context.
///
/// The context is held behind an async mutex because every Provider
/// controller drives its child mutations through `&self` ports while the
/// driver keeps the only `&mut ResourceContext`; the guard is never held
/// across another effect call, so the single-threaded drive order is
/// preserved.
pub(crate) struct ContextChildSurface<'a> {
    ctx: tokio::sync::Mutex<&'a mut ResourceContext>,
    /// The effects the calling driver owns: a Volume or VolumeBinding child
    /// committed here changes the rows the plane's volume root resolver can
    /// see.
    effects: Arc<dyn SharedProviderDriverEffects>,
}

impl<'a> ContextChildSurface<'a> {
    pub(crate) fn new(
        ctx: &'a mut ResourceContext,
        effects: Arc<dyn SharedProviderDriverEffects>,
    ) -> Self {
        Self {
            ctx: tokio::sync::Mutex::new(ctx),
            effects,
        }
    }
}

#[async_trait]
impl SharedProviderChildSurface for ContextChildSurface<'_> {
    async fn ensure(&self, child: ChildEnsure) -> Result<EnsureOutcome, SharedProviderEffectError> {
        let (outcome, committed_row) = {
            let mut ctx = self.ctx.lock().await;
            let outcome = ctx
                .ensure_child(child.clone())
                .await
                .map_err(|_| SharedProviderEffectError::Unavailable)?;
            let committed = matches!(
                outcome,
                EnsureOutcome::Created(_) | EnsureOutcome::Updated(_)
            ) && matches!(child.type_name.as_str(), "Volume" | "VolumeBinding");
            (outcome, committed)
        };
        // The plane's per-resource anchors are a cache of the durable rows:
        // one committed through this endpoint after the plane's last durable
        // load is only observable through a reload, and an unregistered
        // Volume root resolves as `volume-anchor` forever. An `Updated`
        // Volume or VolumeBinding is a new root exactly as a `Created` one is
        // (the controller-bridge path refreshes its registry on every such
        // commit), so the refresh is bounded to both commit shapes.
        if committed_row {
            self.effects.refresh_volume_anchors().await;
        }
        Ok(outcome)
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), SharedProviderEffectError> {
        let mut ctx = self.ctx.lock().await;
        if ctx
            .get(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)?
            .is_none()
        {
            return Ok(());
        }
        ctx.delete(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }

    async fn view(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, SharedProviderEffectError> {
        let mut ctx = self.ctx.lock().await;
        ctx.get_view(key)
            .await
            .map_err(|_| SharedProviderEffectError::Unavailable)
    }
}

// ---------------------------------------------------------------------------
// Per-resource Provider state
// ---------------------------------------------------------------------------

/// In-memory Provider state owned by one driver instance (one resource; R6).
///
/// The old effects kept these in zone-wide maps keyed by resource uid; the
/// driver is already per resource, so the maps hold one slot each. The state
/// is never persisted: after a restart the Provider controllers rehydrate
/// from fresh evidence exactly as the old in-memory maps did.
#[derive(Default)]
pub(crate) struct SharedProviderResourceState {
    /// TPM child-resource controllers (old `tpm_controllers`).
    pub(crate) tpm_controllers: Arc<
        Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_tpm::TpmResourceController>>,
    >,
    /// GPU authority-fenced lifecycle controllers (old `gpu_controllers`).
    pub(crate) gpu_controllers: Arc<
        Mutex<std::collections::BTreeMap<ResourceUid, d2b_provider_device_gpu::GpuController>>,
    >,
    /// GPU authority leases (old `gpu_authority_leases`).
    pub(crate) gpu_authority_leases: Arc<
        Mutex<std::collections::BTreeMap<[u8; 16], d2b_core_controller::authority::AuthorityLease>>,
    >,
}

// ---------------------------------------------------------------------------
// Effect request
// ---------------------------------------------------------------------------

/// Everything one Provider effect call may read from the driver.
pub(crate) struct SharedProviderEffectRequest<'a> {
    pub(crate) zone: ZoneId,
    pub(crate) target: ResourceKey,
    /// The row's durable uid (the Provider effects key on it).
    pub(crate) uid: ResourceUid,
    pub(crate) generation: ResourceGeneration,
    /// Runtime-only operation id (never persisted).
    pub(crate) operation_id: String,
    /// Canonical spec document of the row.
    pub(crate) spec: Value,
    /// Decoded metadata envelope of the row (`ownerRef`, ...).
    pub(crate) metadata: Value,
    /// The driver's last in-memory status projection, when one was published.
    pub(crate) status: Option<Value>,
    /// Provider state owned by the calling driver (per resource).
    pub(crate) state: &'a SharedProviderResourceState,
    /// Manager-routed child mutation surface.
    pub(crate) children: &'a dyn SharedProviderChildSurface,
}

impl SharedProviderEffectRequest<'_> {
    /// The owner reference the old effects read from `/metadata/ownerRef`.
    pub(crate) fn owner_ref(&self) -> Result<ResourceRef, SharedProviderEffectError> {
        owner_ref(&self.metadata)
    }
}

/// Typed Provider effect boundary owned by the d2bd composition root.
///
/// This is the U8 family's dyn-erased port: the driver sees only these
/// closed, typed calls, and the production implementation owns broker
/// dispatch, the pidfd table, authority leases, and the Provider
/// controllers.
#[async_trait]
pub(crate) trait SharedProviderDriverEffects: Send + Sync + 'static {
    /// Reconcile one Network resource through the Network-local controller.
    async fn reconcile_network(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Reconcile one TPM Device through the persistent TPM controller.
    async fn reconcile_tpm(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Reconcile one USBIP resource through its typed lifecycle controller.
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Reconcile one SecurityKey resource through its typed lifecycle
    /// controller.
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Reconcile one GPU Device through the authority-fenced lifecycle.
    async fn reconcile_gpu(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Re-register the plane's per-resource Volume anchors after a provider
    /// effect committed a Volume or VolumeBinding child row.
    ///
    /// The anchor cache is a projection of the durable rows the volume root
    /// resolver reads synchronously, so a row the manager commits through the
    /// child endpoint after the plane's last durable load is otherwise
    /// invisible: its root resolves as `volume-anchor` forever. Families that
    /// declare neither row type keep the default no-op.
    async fn refresh_volume_anchors(&self) {}

    /// Advance one Provider teardown stage (old `execute_finalize`).
    async fn finalize(
        &self,
        kind: SharedProviderKind,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Construction arguments shared by every driver of the family.
#[derive(Clone)]
pub(crate) struct SharedProviderDriverArgs {
    pub(crate) zone: String,
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) effects: Arc<dyn SharedProviderDriverEffects>,
}

/// Factory for the U8 shared host-provider ResourceTypes.
///
/// Construction is infallible by contract (R3).
pub(crate) struct SharedProviderDriverFactory {
    types: Vec<ResourceTypeName>,
    args: SharedProviderDriverArgs,
}

impl SharedProviderDriverFactory {
    pub(crate) fn new(args: SharedProviderDriverArgs) -> Self {
        Self {
            types: SHARED_PROVIDER_TYPES
                .iter()
                .map(|resource_type| ResourceTypeName::new(*resource_type))
                .collect(),
            args,
        }
    }
}

#[async_trait]
impl ResourceDriverFactory for SharedProviderDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(SharedProviderDriver::new(self.args.clone()))
    }
}

/// One desired shared host-provider resource.
pub(crate) struct SharedProviderDriver {
    zone: ZoneId,
    /// The controller generation every effect call binds (KTD7). Each port
    /// receives it inside `SharedProviderEffectRequest`, so only
    /// [`Self::controller_generation`] would read the driver's own copy.
    #[allow(dead_code)]
    controller_generation: ControllerGeneration,
    effects: Arc<dyn SharedProviderDriverEffects>,
    state: Arc<SharedProviderResourceState>,
    /// Dependency/child keys already watched (R12: exactly once per target).
    watched: Vec<ResourceKey>,
}

impl SharedProviderDriver {
    pub(crate) fn new(args: SharedProviderDriverArgs) -> Self {
        let zone = ZoneId::parse(args.zone).expect("driver zone was validated at construction");
        Self {
            zone,
            controller_generation: args.controller_generation,
            effects: args.effects,
            state: Arc::new(SharedProviderResourceState::default()),
            watched: Vec::new(),
        }
    }

    /// The controller generation every effect call binds (KTD7). Read by the
    /// driver's own tests as the constructor's identity check; each production
    /// effect call already carries it on its request.
    #[allow(dead_code)]
    pub(crate) fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    fn error(&self, kind: SharedProviderDriverErrorKind, op: DriverOp) -> SharedProviderDriverError {
        SharedProviderDriverError::new(kind, op)
    }

    /// The decoded spec envelope of the row being driven.
    fn envelope(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<SharedProviderSpecEnvelope, SharedProviderDriverError> {
        ctx.spec::<SharedProviderSpecEnvelope>()
            .cloned()
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// Decoded metadata envelope of the row being driven.
    fn metadata(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<Value, SharedProviderDriverError> {
        decode_metadata(ctx.metadata())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// The family row this row is served by: ResourceType plus the spec's
    /// Provider reference (old `from_registration`).
    fn kind(
        &self,
        ctx: &ResourceContext,
        envelope: &SharedProviderSpecEnvelope,
        op: DriverOp,
    ) -> Result<SharedProviderKind, SharedProviderDriverError> {
        if ctx.key().zone != self.zone.as_str() {
            return Err(self.error(SharedProviderDriverErrorKind::SpecInvalid, op));
        }
        SharedProviderKind::from_type_and_provider(&ctx.key().type_name, envelope.provider_ref())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// Register one internal dependency watch exactly once per target (R12).
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet, and the resync schedule re-evaluates it.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) || target == *ctx.key() {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// The runtime-only operation id of one pass (never persisted).
    fn operation_id(&self, ctx: &ResourceContext, kind: SharedProviderKind) -> String {
        format!(
            "shared-{}-{}-g{}",
            kind.effect_id(),
            ctx.uid()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            ctx.generation(),
        )
    }

    fn child_key(&self, target: &ResourceRef) -> ResourceKey {
        ResourceKey::new(
            self.zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
    }

    /// The desired child set of one reconcile pass, as manager child rows.
    ///
    /// Providers declare the children they own; the driver materializes them
    /// into the manager's child shape (Core-owned bodies, F1).
    ///
    /// `None` marks a kind whose children are not this driver's to declare
    /// *or diff*: the Device/Service kinds own rows another layer declares -
    /// the Zone bundle declares the Device-owned worker rows
    /// (`Process/swtpm-<device>`, `Process/gpu-<device>`, KTD13) with `Nix`
    /// provenance, and the family's Provider effect ensures its own
    /// controller-created rows through the child surface (the TPM state
    /// Volume). Running the obsolete-children diff against an empty desired
    /// set would retire every declared row on the first reconcile pass - the
    /// bundle ingest would look like it never happened. Those kinds retire
    /// their whole owned subtree on teardown instead
    /// ([`Self::retire_obsolete_children`] in `delete`).
    async fn desired_children(
        &self,
        ctx: &mut ResourceContext,
        kind: SharedProviderKind,
        envelope: &SharedProviderSpecEnvelope,
    ) -> Result<Option<Vec<ChildEnsure>>, SharedProviderDriverError> {
        let op = DriverOp::Reconcile;
        let owner = crate::shared_provider_driver::key_ref(ctx.key());
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let spec = envelope.value();
        match kind {
            SharedProviderKind::Network => {
                let network_spec = self.network_spec(spec, op)?;
                Ok(Some(network_child_ensures(&owner, &uid, &network_spec, op)?))
            }
            SharedProviderKind::UsbipBinding => {
                let service_ref = spec_ref(spec, "/spec/serviceRef")
                    .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                let guest_ref = spec_ref(spec, "/spec/guestRef")
                    .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                let desired = d2b_provider_device_usbip::binding_child_resources(
                    &owner, &service_ref, &guest_ref,
                )
                .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                Ok(Some(self.binding_child_ensures(&desired, op)?))
            }
            SharedProviderKind::SecurityKeyService => {
                let device_uid = match spec_ref(spec, "/spec/provider/settings/deviceRef") {
                    Ok(device_ref) => {
                        let key = self.child_key(&device_ref);
                        let row = ctx
                            .get(&key)
                            .await
                            .map_err(|_| {
                                self.error(SharedProviderDriverErrorKind::ChildMutation, op)
                            })?
                            .filter(|row| !row.deleting);
                        row.map(|row| row.uid)
                    }
                    Err(_) => None,
                };
                let Some(device_uid) = device_uid else {
                    // The Device row the relay is derived from is not present:
                    // the Service effect reports Pending until it is; no child
                    // is declared yet (old effect returned Pending here).
                    return Ok(Some(Vec::new()));
                };
                let device_uid = resource_uid(&device_uid)
                    .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                Ok(Some(security_key_relay_child_ensures(spec, &owner, &device_uid, op)?))
            }
            SharedProviderKind::SecurityKeyBinding => {
                let service_ref = spec_ref(spec, "/spec/serviceRef")
                    .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                let target_ref = spec
                    .pointer("/spec/target/guestRef")
                    .or_else(|| spec.pointer("/spec/guestRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok())
                    .ok_or_else(|| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                let user_ref = spec
                    .pointer("/spec/target/userRef")
                    .or_else(|| spec.pointer("/spec/userRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok());
                let desired = match user_ref {
                    Some(user_ref) => {
                        d2b_provider_device_security_key::SecurityKeyController::child_resources_for_user(
                            &owner, &service_ref, &target_ref, &user_ref,
                        )
                    }
                    None => d2b_provider_device_security_key::SecurityKeyController::child_resources(
                        &owner, &service_ref, &target_ref,
                    ),
                }
                .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
                Ok(Some(self.binding_child_ensures(&desired, op)?))
            }
            // The Device/Service kinds declare no children through the
            // driver; see [`Self::desired_children`]'s contract.
            SharedProviderKind::TpmDevice
            | SharedProviderKind::UsbipDevice
            | SharedProviderKind::UsbipService
            | SharedProviderKind::SecurityKeyDevice
            | SharedProviderKind::GpuDevice => Ok(None),
        }
    }

    fn network_spec(
        &self,
        spec: &Value,
        op: DriverOp,
    ) -> Result<NetworkSpec, SharedProviderDriverError> {
        let mut spec_value = spec.clone();
        if let Some(spec) = spec_value.as_object_mut() {
            for field in ["providerRef", "updatePolicy", "provider"] {
                spec.remove(field);
            }
        }
        serde_json::from_value(spec_value)
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))
    }

    /// Materialize one Provider-declared Binding child set into manager rows
    /// (old Core `materialize_child_create_payload`: Providers declare
    /// intent, Core owns the child body, and the Process Provider stays
    /// Core-chosen).
    fn binding_child_ensures(
        &self,
        desired: &d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildSet,
        op: DriverOp,
    ) -> Result<Vec<ChildEnsure>, SharedProviderDriverError> {
        desired
            .iter()
            .map(|intent| {
                binding_child_ensure(intent, &self.zone, op).map_err(|kind| self.error(kind, op))
            })
            .collect()
    }

    /// Retire owned children the desired set no longer derives, in the
    /// family's preserved order (endpoint-first, process-last; R9/F3).
    async fn retire_obsolete_children(
        &self,
        ctx: &mut ResourceContext,
        desired: &[ChildEnsure],
        op: DriverOp,
    ) -> Result<bool, SharedProviderDriverError> {
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(SharedProviderDriverErrorKind::ChildMutation, op))?;
        let mut obsolete = owned
            .iter()
            .filter(|row| {
                !row.deleting
                    && !desired
                        .iter()
                        .any(|child| owned_child_matches_child_ensure(row, child))
            })
            .collect::<Vec<_>>();
        obsolete.sort_by_key(|row| (teardown_rank(&row.key.type_name), row.key.name.clone()));
        let mut mutated = false;
        for row in obsolete {
            ctx.delete(&row.key)
                .await
                .map_err(|_| self.error(SharedProviderDriverErrorKind::ChildMutation, op))?;
            mutated = true;
        }
        Ok(mutated)
    }

    fn effect_error(
        &self,
        error: SharedProviderEffectError,
        op: DriverOp,
    ) -> SharedProviderDriverError {
        match error {
            SharedProviderEffectError::InvalidResource => {
                self.error(SharedProviderDriverErrorKind::SpecInvalid, op)
            }
            SharedProviderEffectError::Pending => {
                self.error(SharedProviderDriverErrorKind::FinalizePending, op)
            }
            SharedProviderEffectError::Unavailable => {
                self.error(SharedProviderDriverErrorKind::ProviderUnavailable, op)
            }
        }
    }

    /// Dispatch one effect call to its typed Provider row.
    async fn run_effect(
        &self,
        kind: SharedProviderKind,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderDriverError> {
        let result = match kind {
            SharedProviderKind::Network => self.effects.reconcile_network(request).await,
            SharedProviderKind::TpmDevice => self.effects.reconcile_tpm(request).await,
            SharedProviderKind::UsbipDevice
            | SharedProviderKind::UsbipService
            | SharedProviderKind::UsbipBinding => {
                let component = kind.usbip_component().expect("USBIP kind has a component");
                self.effects.reconcile_usbip(component, request).await
            }
            SharedProviderKind::SecurityKeyDevice
            | SharedProviderKind::SecurityKeyService
            | SharedProviderKind::SecurityKeyBinding => {
                let component = kind
                    .security_key_component()
                    .expect("SecurityKey kind has a component");
                self.effects
                    .reconcile_security_key(component, request)
                    .await
            }
            SharedProviderKind::GpuDevice => self.effects.reconcile_gpu(request).await,
        };
        result.map_err(|error| self.effect_error(error, DriverOp::Reconcile))
    }
}

impl core::fmt::Debug for SharedProviderDriver {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SharedProviderDriver")
            .field("zone", &self.zone)
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for SharedProviderDriverFactory {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SharedProviderDriverFactory")
            .field("types", &self.types)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ResourceDriver for SharedProviderDriver {
    type Error = SharedProviderDriverError;

    fn classify_error(&self, error: &SharedProviderDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Structural validation (old `validate_spec`): the stored spec decodes
    /// and names a Provider this family owns for the row's ResourceType.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let envelope = self.envelope(ctx, DriverOp::Validate)?;
        let _ = self.kind(ctx, &envelope, DriverOp::Validate)?;
        Ok(())
    }

    /// Discovery and adoption on the realization target (F2): the family's
    /// child-bearing kinds adopt when their complete desired child set is
    /// already present and current; kinds that realize nothing through
    /// resource rows adopt their Provider-side realization in reconcile.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let envelope = self.envelope(ctx, op)?;
        let kind = self.kind(ctx, &envelope, op)?;
        let Some(desired) = self.desired_children(ctx, kind, &envelope).await? else {
            // A kind with no driver-declared children has nothing to adopt
            // here; its Provider-side realization is discovered in the
            // effect.
            return Ok(RecoveryOutcome::Adopted);
        };
        if desired.is_empty() {
            return Ok(RecoveryOutcome::Adopted);
        }
        let owned = ctx
            .children()
            .await
            .map_err(|_| self.error(SharedProviderDriverErrorKind::ChildMutation, op))?;
        let current = desired.iter().all(|child| {
            owned
                .iter()
                .any(|row| owned_child_matches_child_ensure(row, child) && !row.deleting)
        });
        Ok(if current {
            RecoveryOutcome::Adopted
        } else {
            RecoveryOutcome::Missing
        })
    }

    /// One reconcile pass (old `plan` + `reconcile` + `execute_effect`).
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        let envelope = self.envelope(ctx, op)?;
        let kind = self.kind(ctx, &envelope, op)?;
        let metadata = self.metadata(ctx, op)?;

        // Dependency edges (R12/R17): the resources this family's effects
        // read are watched so their readiness or death wakes this actor.
        for dependency in declared_dependency_refs(kind, envelope.value(), &metadata) {
            self.watch_once(ctx, self.child_key(&dependency)).await;
        }

        // Desired child set through the manager child API (F1): every row is
        // committed before its actor exists. Kinds whose children another
        // layer declares (the bundle's Device worker rows, the effect's
        // controller-created rows) have no desired set here and skip the
        // child machinery entirely - diffing their owned rows against an
        // empty set would retire the declared rows on every pass.
        let desired = self.desired_children(ctx, kind, &envelope).await?;
        let mut mutated = false;
        if let Some(desired) = &desired {
            for child in desired {
                match ctx.ensure_child(child.clone()).await {
                    Ok(EnsureOutcome::Created(_)) | Ok(EnsureOutcome::Updated(_)) => mutated = true,
                    Ok(EnsureOutcome::Unchanged(_)) => {}
                    Err(_) => {
                        return Err(self.error(SharedProviderDriverErrorKind::ChildMutation, op));
                    }
                }
            }
            mutated |= self.retire_obsolete_children(ctx, desired, op).await?;
            for child in desired {
                self.watch_once(
                    ctx,
                    ResourceKey::new(
                        self.zone.as_str(),
                        child.type_name.as_str(),
                        child.name.as_str(),
                    ),
                )
                .await;
            }
        }

        let operation_id = self.operation_id(ctx, kind);
        let target = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let status = ctx
            .status::<SharedProviderDriverStatus>()
            .and_then(|status| status.resource.clone());
        // The surface borrows the context mutably for the effect call; the
        // driver reads nothing else from the context until it is dropped.
        let outcome = {
            let surface = ContextChildSurface::new(ctx, Arc::clone(&self.effects));
            let request = SharedProviderEffectRequest {
                zone: self.zone.clone(),
                target,
                uid,
                generation,
                operation_id,
                spec: envelope.value().clone(),
                metadata: metadata.clone(),
                status,
                state: &self.state,
                children: &surface,
            };
            self.run_effect(kind, &request).await?
        };

        ctx.set_status(SharedProviderDriverStatus {
            phase: outcome.phase,
            resource: outcome.resource_projection.clone(),
        });
        if outcome.phase != SharedProviderEffectPhase::Ready || mutated {
            ctx.requeue_after(kind.resync());
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Drain step (R10, F3): every owned child finalizes before this
    /// resource's own teardown, and before the Provider's teardown stage in
    /// [`ResourceDriver::delete`]. The call nudges each owned child through
    /// its own finalize-before-delete pass and requeues this pass while any
    /// child row is still live. Idempotent under retry.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        ctx.finalize_owned_resources()
            .await
            .map_err(|_| self.error(SharedProviderDriverErrorKind::FinalizePending, DriverOp::Delete))?;
        Ok(())
    }

    /// Teardown (old `prepare_finalize` + `execute_finalize` + `finalize`):
    /// the Provider's teardown stage runs first, then the owned children
    /// retire in the family's preserved order. Idempotent under retry (R10).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Delete;
        let Ok(envelope) = self.envelope(ctx, op) else {
            return Ok(());
        };
        let kind = self.kind(ctx, &envelope, op)?;
        let metadata = self.metadata(ctx, op)?;
        let operation_id = self.operation_id(ctx, kind);
        let target = ctx.key().clone();
        let uid = resource_uid(ctx.uid())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let generation = ResourceGeneration::new(ctx.generation())
            .map_err(|_| self.error(SharedProviderDriverErrorKind::SpecInvalid, op))?;
        let status = ctx
            .status::<SharedProviderDriverStatus>()
            .and_then(|status| status.resource.clone());
        let finalized = {
            let surface = ContextChildSurface::new(ctx, Arc::clone(&self.effects));
            let request = SharedProviderEffectRequest {
                zone: self.zone.clone(),
                target,
                uid,
                generation,
                operation_id,
                spec: envelope.value().clone(),
                metadata,
                status,
                state: &self.state,
                children: &surface,
            };
            self.effects
                .finalize(kind, &request)
                .await
                .map_err(|error| self.effect_error(error, op))?
        };
        if finalized == SharedProviderFinalize::Pending {
            return Err(self.error(SharedProviderDriverErrorKind::FinalizePending, op));
        }
        self.retire_obsolete_children(ctx, &[], op).await?;
        Ok(())
    }
}

/// The resource reference of one manager key.
pub(crate) fn key_ref(key: &ResourceKey) -> ResourceRef {
    ResourceRef::parse(&format!("{}/{}", key.type_name, key.name))
        .expect("manager keys carry canonical resource references")
}

/// The child row key a `ChildEnsure` derives (manager identity rules).
fn owned_child_matches_child_ensure(row: &StoredDesiredResource, child: &ChildEnsure) -> bool {
    row.key.type_name == child.type_name.as_str() && row.key.name == child.name
}

/// Old `BindingChildKind` teardown ranks: endpoints retire before their
/// producing processes.
fn teardown_rank(resource_type: &str) -> u8 {
    match resource_type {
        "Endpoint" => 0,
        "EphemeralProcess" => 1,
        _ => 2,
    }
}

fn spec_ref(spec: &Value, path: &str) -> Result<ResourceRef, ()> {
    spec.pointer(path)
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or(())
}

/// One Provider-declared Binding child as a manager child row.
fn binding_child_ensure(
    intent: &BindingChildIntent,
    zone: &ZoneId,
    _op: DriverOp,
) -> Result<ChildEnsure, SharedProviderDriverErrorKind> {
    let payload = d2b_core_controller::materialize_child_create_payload(intent, zone)
        .map_err(|_| SharedProviderDriverErrorKind::SpecInvalid)?;
    let value = serde_json::from_slice::<Value>(&payload)
        .map_err(|_| SharedProviderDriverErrorKind::SpecInvalid)?;
    let spec = value
        .get("spec")
        .cloned()
        .ok_or(SharedProviderDriverErrorKind::SpecInvalid)?;
    let metadata = json!({
        "ownerRef": intent.owner_ref().to_canonical_string(),
        "labels": {},
        "annotations": {},
    });
    Ok(ChildEnsure {
        type_name: ResourceTypeName::new(intent.kind().resource_type()),
        name: intent.resource_ref().name().as_str().to_owned(),
        spec: serde_json::to_vec(&spec).map_err(|_| SharedProviderDriverErrorKind::SpecInvalid)?,
        metadata: serde_json::to_vec(&metadata)
            .map_err(|_| SharedProviderDriverErrorKind::SpecInvalid)?,
    })
}

/// The Network family's desired children: the config Volume, the net-VM
/// Guest, and the in-guest network agent Process (old
/// `SharedRunnerNetworkResources` derived refs, unchanged).
fn network_child_ensures(
    owner: &ResourceRef,
    network_uid: &ResourceUid,
    spec: &NetworkSpec,
    op: DriverOp,
) -> Result<Vec<ChildEnsure>, SharedProviderDriverError> {
    let error = |kind: SharedProviderDriverErrorKind| SharedProviderDriverError::new(kind, op);
    let vm_name = d2b_provider_network_local::ifname::derive_network_child_name(network_uid, "vm");
    let agent_name =
        d2b_provider_network_local::ifname::derive_network_child_name(network_uid, "agent");
    let metadata = serde_json::to_vec(&json!({
        "ownerRef": owner.to_canonical_string(),
        "labels": {},
        "annotations": {},
    }))
    .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;

    let volume_spec =
        d2b_provider_network_local::controller::config_volume_spec("host-system", Some(&vm_name))
            .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let mut volume = serde_json::to_value(&volume_spec)
        .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    volume
        .as_object_mut()
        .ok_or_else(|| error(SharedProviderDriverErrorKind::SpecInvalid))?
        .insert(
            "providerRef".to_owned(),
            Value::String("Provider/volume-local".to_owned()),
        );

    let artifact = d2b_provider_network_local::artifact::resolve_net_vm_system_artifact(
        spec,
        &[d2b_provider_network_local::artifact::ArtifactCatalogEntry::new(
            spec.net_vm_system_artifact_id().clone(),
            d2b_provider_network_local::artifact::ArtifactKind::NixosSystem,
        )],
    )
    .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let guest = GuestSpec::new(ExecutionPolicy::system_default(), Some(artifact));
    let mut guest_value = serde_json::to_value(&guest)
        .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    guest_value
        .as_object_mut()
        .ok_or_else(|| error(SharedProviderDriverErrorKind::SpecInvalid))?
        .insert(
            "providerRef".to_owned(),
            Value::String("Provider/runtime-cloud-hypervisor".to_owned()),
        );

    let agent = d2b_provider_network_local::controller::guest_agent_process_spec(&vm_name)
        .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let mut agent_value = serde_json::to_value(&agent)
        .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    agent_value
        .as_object_mut()
        .ok_or_else(|| error(SharedProviderDriverErrorKind::SpecInvalid))?
        .insert(
            "providerRef".to_owned(),
            Value::String("Provider/system-minijail".to_owned()),
        );

    Ok(vec![
        ChildEnsure {
            type_name: ResourceTypeName::new("Volume"),
            name: "net-config".to_owned(),
            spec: serde_json::to_vec(&volume)
                .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?,
            metadata: metadata.clone(),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Guest"),
            name: vm_name.clone(),
            spec: serde_json::to_vec(&guest_value)
                .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?,
            metadata: metadata.clone(),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Process"),
            name: agent_name,
            spec: serde_json::to_vec(&agent_value)
                .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?,
            metadata,
        },
    ])
}

/// The SecurityKey Service's desired children: the Host relay Process and the
/// relay Endpoint it produces (old inline specs in `reconcile_security_key`,
/// unchanged).
fn security_key_relay_child_ensures(
    spec: &Value,
    owner: &ResourceRef,
    device_uid: &ResourceUid,
    op: DriverOp,
) -> Result<Vec<ChildEnsure>, SharedProviderDriverError> {
    let error = |kind: SharedProviderDriverErrorKind| SharedProviderDriverError::new(kind, op);
    let settings = spec
        .pointer("/spec/provider/settings")
        .ok_or_else(|| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let device_ref = settings
        .get("deviceRef")
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or_else(|| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let relay_endpoint_ref = settings
        .get("relayEndpointRef")
        .and_then(Value::as_str)
        .and_then(|value| ResourceRef::parse(value).ok())
        .ok_or_else(|| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let relay_process_name = d2b_provider_device_security_key::security_key_process_name(
        device_uid,
        d2b_provider_device_security_key::SecurityKeyProcessRole::HostRelay,
    )
    .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let relay_process_ref = ResourceRef::parse(&format!("Process/{relay_process_name}"))
        .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let metadata = serde_json::to_vec(&json!({
        "ownerRef": owner.to_canonical_string(),
        "labels": {},
        "annotations": {},
    }))
    .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?;
    let relay_process_spec = json!({
        "providerRef": "Provider/system-minijail",
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
        "providerRef": d2b_provider_device_security_key::PROVIDER_REF,
        "producerRef": relay_process_ref.to_canonical_string(),
        "endpointClass": "device",
        "transport": "vsock",
        "purpose": "device-security-key.d2bus.org/ctaphid-relay",
        "serviceFingerprint": "device-security-key.d2bus.org/SecurityKeyCtapRelay.v3",
        "locality": "cross-domain",
        "visibility": "zone",
        "attachmentPolicy": "component-session",
        "consumerPolicy": {
            "allowedProviderComponents": ["device-security-key.d2bus.org/frontend"],
            "allowedOperations": ["resolve"]
        },
        "lifecyclePolicy": "recycle-with-producer"
    });
    let _ = owner;
    Ok(vec![
        ChildEnsure {
            type_name: ResourceTypeName::new("Process"),
            name: relay_process_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&relay_process_spec)
                .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?,
            metadata: metadata.clone(),
        },
        ChildEnsure {
            type_name: ResourceTypeName::new("Endpoint"),
            name: relay_endpoint_ref.name().as_str().to_owned(),
            spec: serde_json::to_vec(&endpoint_spec)
                .map_err(|_| error(SharedProviderDriverErrorKind::SpecInvalid))?,
            metadata,
        },
    ])
}

/// The dependency references one family row declares, in the order the old
/// Runner's dependency selectors carried them (R12/R17 watch targets).
pub(crate) fn declared_dependency_refs(
    kind: SharedProviderKind,
    spec: &Value,
    metadata: &Value,
) -> Vec<ResourceRef> {
    let mut refs = Vec::new();
    let mut push = |reference: Option<ResourceRef>| {
        if let Some(reference) = reference {
            refs.push(reference);
        }
    };
    match kind {
        SharedProviderKind::Network => {
            if let Some(attachments) = spec.pointer("/spec/attachments").and_then(Value::as_array) {
                for attachment in attachments {
                    push(
                        attachment
                            .get("executionRef")
                            .and_then(Value::as_str)
                            .and_then(|value| ResourceRef::parse(value).ok()),
                    );
                }
            }
        }
        SharedProviderKind::TpmDevice | SharedProviderKind::GpuDevice => {
            push(owner_ref(metadata).ok());
        }
        SharedProviderKind::UsbipDevice => {}
        SharedProviderKind::UsbipService => push(spec_ref(spec, "/spec/backingDeviceRef").ok()),
        SharedProviderKind::UsbipBinding => {
            push(spec_ref(spec, "/spec/serviceRef").ok());
            push(spec_ref(spec, "/spec/guestRef").ok());
        }
        SharedProviderKind::SecurityKeyDevice => {}
        SharedProviderKind::SecurityKeyService => {
            push(
                spec.pointer("/spec/provider/settings/deviceRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok()),
            );
            push(
                spec.pointer("/spec/provider/settings/relayEndpointRef")
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok()),
            );
        }
        SharedProviderKind::SecurityKeyBinding => {
            push(spec_ref(spec, "/spec/serviceRef").ok());
            push(
                spec.pointer("/spec/target/guestRef")
                    .or_else(|| spec.pointer("/spec/guestRef"))
                    .and_then(Value::as_str)
                    .and_then(|value| ResourceRef::parse(value).ok()),
            );
        }
    }
    refs
}

// ---------------------------------------------------------------------------
// Tests: driver behavior over recording effects and a recording manager
// endpoint (the shape the U7/U12 lanes used). The fakes record the exact
// ordering the driver promises: manager child mutations before the typed
// effect, endpoint-first/process-last retirement, and the effect/teardown
// sequence on delete.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, SpecDecoder,
        WatchId, WatchRegistration,
    };
    use d2b_resource_runtime::driver::{DynResourceDriver, RecoveryOutcome, ResourceDriverFactory};
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{
        ResourceKey, ResourceProvenance, StoredDesiredResource,
    };
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;
    use serde_json::json;

    use super::{
        ContextChildSurface, DEVICE_TYPE_NAME, NETWORK_TYPE_NAME, SharedProviderChildSurface,
        SharedProviderDriverArgs, SharedProviderDriverFactory, SharedProviderEffectError,
        SharedProviderEffectOutcome, SharedProviderEffectPhase, SharedProviderEffectRequest,
        SharedProviderFinalize, SharedProviderKind, shared_provider_spec_decoder,
    };

    /// Ordered log every fake writes to, so ordering is one assertion.
    type Log = Arc<parking_lot::Mutex<Vec<String>>>;

    struct RecordingManager {
        log: Log,
        owned: parking_lot::Mutex<Vec<StoredDesiredResource>>,
        /// Scripted ensure outcomes, consumed in order; empty falls back to a
        /// `Created` row.
        ensure_outcomes: parking_lot::Mutex<std::collections::VecDeque<EnsureOutcome>>,
    }

    impl RecordingManager {
        fn new(log: Log) -> Arc<Self> {
            Arc::new(Self {
                log,
                owned: parking_lot::Mutex::new(Vec::new()),
                ensure_outcomes: parking_lot::Mutex::new(std::collections::VecDeque::new()),
            })
        }

        fn with_owned(log: Log, owned: Vec<StoredDesiredResource>) -> Arc<Self> {
            Arc::new(Self {
                log,
                owned: parking_lot::Mutex::new(owned),
                ensure_outcomes: parking_lot::Mutex::new(std::collections::VecDeque::new()),
            })
        }

        fn script_ensure_outcomes(&self, outcomes: Vec<EnsureOutcome>) {
            self.ensure_outcomes.lock().extend(outcomes);
        }
    }

    #[async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.log.lock().push(format!(
                "ensure:{}/{}",
                child.type_name.as_str(),
                child.name
            ));
            if let Some(outcome) = self.ensure_outcomes.lock().pop_front() {
                return Ok(outcome);
            }
            Ok(EnsureOutcome::Created(test_row(
                "dev",
                child.type_name.as_str(),
                &child.name,
            )))
        }

        async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
            Ok(self
                .owned
                .lock()
                .iter()
                .find(|row| row.key == *key)
                .cloned())
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            // Desired rows only: this fixture publishes no runtime status, so
            // it serves no observed state.
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log.lock().push(format!("delete:{}/{}", key.type_name, key.name));
            self.owned.lock().retain(|row| row.key != *key);
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            Ok(self.owned.lock().clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingRequeue {
        scheduled: parking_lot::Mutex<Vec<RequeueId>>,
    }

    impl RequeueScheduler for RecordingRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            let mut scheduled = self.scheduled.lock();
            let id = RequeueId(scheduled.len() as u64 + 1);
            scheduled.push(id);
            id
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    struct RecordingEffects {
        log: Log,
        phase: parking_lot::Mutex<SharedProviderEffectPhase>,
        finalize: parking_lot::Mutex<SharedProviderFinalize>,
        /// Volume-anchor reloads the child surface requested.
        refreshes: parking_lot::Mutex<usize>,
    }

    impl RecordingEffects {
        fn new(log: Log, phase: SharedProviderEffectPhase, finalize: SharedProviderFinalize) -> Arc<Self> {
            Arc::new(Self {
                log,
                phase: parking_lot::Mutex::new(phase),
                finalize: parking_lot::Mutex::new(finalize),
                refreshes: parking_lot::Mutex::new(0),
            })
        }

        fn refreshes(&self) -> usize {
            *self.refreshes.lock()
        }
    }

    #[async_trait]
    impl crate::shared_provider_driver::SharedProviderDriverEffects for RecordingEffects {
        async fn reconcile_network(
            &self,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            self.log.lock().push("effect:network".to_owned());
            Ok(SharedProviderEffectOutcome::projection(
                *self.phase.lock(),
                json!({"seen": true}),
            ))
        }

        async fn reconcile_tpm(
            &self,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            self.log.lock().push("effect:tpm".to_owned());
            Ok(SharedProviderEffectOutcome::phase(*self.phase.lock()))
        }

        async fn reconcile_usbip(
            &self,
            component: crate::shared_provider_driver::UsbipComponent,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            self.log.lock().push(format!("effect:usbip:{component:?}"));
            Ok(SharedProviderEffectOutcome::phase(*self.phase.lock()))
        }

        async fn reconcile_security_key(
            &self,
            component: crate::shared_provider_driver::SecurityKeyComponent,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            self.log.lock().push(format!("effect:security-key:{component:?}"));
            Ok(SharedProviderEffectOutcome::phase(*self.phase.lock()))
        }

        async fn reconcile_gpu(
            &self,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError> {
            self.log.lock().push("effect:gpu".to_owned());
            Ok(SharedProviderEffectOutcome::phase(*self.phase.lock()))
        }

        async fn finalize(
            &self,
            kind: SharedProviderKind,
            _request: &SharedProviderEffectRequest<'_>,
        ) -> Result<SharedProviderFinalize, SharedProviderEffectError> {
            self.log.lock().push(format!("finalize:{}", kind.effect_id()));
            Ok(*self.finalize.lock())
        }

        async fn refresh_volume_anchors(&self) {
            *self.refreshes.lock() += 1;
        }
    }

    fn test_row(zone: &str, type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new(zone, type_name, name),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: b"spec".to_vec(),
            metadata: Vec::new(),
            created_at: 1_725_000_000,
        }
    }

    fn owned_row(zone: &str, type_name: &str, name: &str) -> StoredDesiredResource {
        StoredDesiredResource {
            owner_uid: Some([0x42; 16]),
            ..test_row(zone, type_name, name)
        }
    }

    struct Fixture {
        ctx: ResourceContext,
        effects: Arc<RecordingEffects>,
        requeue: Arc<RecordingRequeue>,
        log: Log,
    }

    fn fixture(
        type_name: &str,
        name: &str,
        spec: serde_json::Value,
        manager: Arc<RecordingManager>,
        requeue: Arc<RecordingRequeue>,
        log: Log,
        phase: SharedProviderEffectPhase,
        finalize: SharedProviderFinalize,
    ) -> Fixture {
        let mut row = test_row("dev", type_name, name);
        row.spec = serde_json::to_vec(&spec).expect("spec json");
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let effects = RecordingEffects::new(Arc::clone(&log), phase, finalize);
        let ctx = ResourceContext::new(
            row,
            TargetHandle::Host,
            decoder(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
            Arc::clone(&requeue) as Arc<dyn RequeueScheduler>,
            effects_tx,
            notify_tx,
        );
        Fixture {
            ctx,
            effects,
            requeue,
            log,
        }
    }

    fn decoder() -> Arc<dyn SpecDecoder> {
        shared_provider_spec_decoder()
    }

    async fn driver(fixture: &Fixture) -> Box<dyn DynResourceDriver> {
        let factory = SharedProviderDriverFactory::new(SharedProviderDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
                .expect("generation"),
            effects: Arc::clone(&fixture.effects)
                as Arc<dyn crate::shared_provider_driver::SharedProviderDriverEffects>,
        });
        let key = fixture.ctx.key().clone();
        factory.create(&key).await
    }

    /// Device spec pinned to one Device Provider.
    fn device_spec(provider_ref: &str) -> serde_json::Value {
        json!({
            "name": "dev-row",
            "providerRef": provider_ref,
        })
    }

    #[test]
    fn factory_registers_the_six_family_resource_types() {
        let factory = SharedProviderDriverFactory::new(SharedProviderDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
                .expect("generation"),
            effects: RecordingEffects::new(
                Arc::new(parking_lot::Mutex::new(Vec::new())),
                SharedProviderEffectPhase::Pending,
                SharedProviderFinalize::Complete,
            ),
        });
        let types = factory
            .resource_types()
            .iter()
            .map(|resource_type| resource_type.as_str().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                NETWORK_TYPE_NAME.to_owned(),
                DEVICE_TYPE_NAME.to_owned(),
                d2b_provider_device_usbip::USB_SERVICE_RESOURCE_TYPE.to_owned(),
                d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE.to_owned(),
                d2b_provider_device_security_key::SECURITY_KEY_SERVICE_RESOURCE_TYPE.to_owned(),
                d2b_provider_device_security_key::SECURITY_KEY_BINDING_RESOURCE_TYPE.to_owned(),
            ]
        );
    }

    /// The child surface's volume-anchor reload: a `Volume`/`VolumeBinding`
    /// child committed after the plane's last durable load is only observable
    /// through a reload, and an `Updated` child is a new root exactly as a
    /// `Created` one is (a spec-less update still changes the row the anchors
    /// resolve from). Other child types and an `Unchanged` ensure change
    /// nothing, so they never reload.
    #[tokio::test]
    async fn volume_anchor_refresh_covers_created_and_updated_volume_children() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut f = fixture(
            "Volume",
            "vol-data",
            json!({"providerRef": "Provider/volume-local"}),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            log,
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        manager.script_ensure_outcomes(vec![
            EnsureOutcome::Created(test_row("dev", "Volume", "vol-data")),
            EnsureOutcome::Updated(test_row("dev", "Volume", "vol-data")),
            EnsureOutcome::Unchanged(test_row("dev", "Volume", "vol-data")),
            EnsureOutcome::Updated(test_row("dev", "Network", "net-data")),
        ]);
        let child = |type_name: &str, name: &str| ChildEnsure {
            type_name: d2b_resource_runtime::identity::ResourceTypeName::new(type_name),
            name: name.to_owned(),
            spec: b"{}".to_vec(),
            metadata: Vec::new(),
        };

        let surface = ContextChildSurface::new(
            &mut f.ctx,
            Arc::clone(&f.effects) as Arc<dyn crate::shared_provider_driver::SharedProviderDriverEffects>,
        );
        surface.ensure(child("Volume", "vol-data")).await.expect("created");
        assert_eq!(f.effects.refreshes(), 1, "a created Volume reloads the anchors");
        surface.ensure(child("Volume", "vol-data")).await.expect("updated");
        assert_eq!(f.effects.refreshes(), 2, "an updated Volume reloads the anchors");
        surface.ensure(child("Volume", "vol-data")).await.expect("unchanged");
        assert_eq!(f.effects.refreshes(), 2, "an unchanged ensure reloads nothing");
        surface.ensure(child("Network", "net-data")).await.expect("other type");
        assert_eq!(f.effects.refreshes(), 2, "only Volume/VolumeBinding commits reload");
    }

    /// A Device row naming a Provider outside the family is terminal: the
    /// stored spec can never be served by this factory (old
    /// `from_registration` refusal).
    #[tokio::test]
    async fn validate_rejects_a_device_provider_outside_the_family() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec("Provider/volume-local"),
            RecordingManager::new(Arc::clone(&log)),
            Arc::new(RecordingRequeue::default()),
            log,
            SharedProviderEffectPhase::Pending,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        let failure = driver.validate(&mut fixture.ctx).await.expect_err("must refuse");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::terminal(
                d2b_resource_runtime::error::DriverOp::Validate
            )
        );
    }

    /// The typed effect runs through the port, the in-memory status carries
    /// its projection (R11), and a not-converged reconcile self-requeues.
    #[tokio::test]
    async fn reconcile_runs_the_effect_then_publishes_status_and_requeues_while_pending() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            RecordingManager::new(Arc::clone(&log)),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Pending,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        assert!(driver.validate(&mut fixture.ctx).await.is_ok());
        let outcome = driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        assert_eq!(outcome, d2b_resource_runtime::driver::ReconcileOutcome::Satisfied);

        let log = fixture.log.lock().clone();
        let effect_at = log.iter().position(|entry| entry == "effect:tpm").expect("effect ran");
        assert!(!log[..effect_at].iter().any(|entry| entry.starts_with("delete:")), "{log:?}");

        let status = fixture
            .ctx
            .status::<super::SharedProviderDriverStatus>()
            .expect("status published (R11)");
        assert_eq!(status.phase(), "Pending");
        // The typed effect's projection is published verbatim; the tpm arm
        // returns a phase-only outcome.
        assert_eq!(status.resource, None);
        assert_eq!(fixture.requeue.scheduled.lock().len(), 1, "pending reconcile self-resyncs");
    }

    /// Owned children retire in the family's preserved order on teardown:
    /// endpoints before the processes they gate (R9/F3). The Device kinds'
    /// rows are declared by another layer, so teardown is the pass that
    /// retires their whole owned subtree.
    #[tokio::test]
    async fn delete_retires_owned_children_endpoint_first() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![
                owned_row("dev", "Process", "stale-proxy"),
                owned_row("dev", "Endpoint", "stale-endpoint"),
            ],
        );
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        driver.delete(&mut fixture.ctx).await.expect("teardown");
        let deletions = log
            .lock()
            .iter()
            .filter(|entry| entry.starts_with("delete:"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            deletions,
            vec!["delete:Endpoint/stale-endpoint".to_owned(), "delete:Process/stale-proxy".to_owned()]
        );
    }

    /// The Device kinds own rows another layer declares - the Zone bundle's
    /// Device worker rows (`Process/swtpm-<device>`, `EphemeralProcess/swtpm-flush-<device>`,
    /// KTD13) and the family effect's controller-created rows (the TPM state
    /// Volume) - and the driver derives no child set for them. A reconcile
    /// pass must leave those rows alone: diffing the owned set against the
    /// empty desired set retired every declared row on the pass that
    /// followed the bundle ingest, so the ingest's rows never stayed in the
    /// manager projection the `device-worker-launch` fixture reads.
    #[tokio::test]
    async fn reconcile_leaves_the_declared_device_rows_alone() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![
                owned_row("dev", "Process", "swtpm-tpm-0"),
                owned_row("dev", "EphemeralProcess", "swtpm-flush-tpm-0"),
                owned_row("dev", "Endpoint", "tpm-ctrl-tpm-0"),
            ],
        );
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let entries = log.lock().clone();
        assert!(
            !entries.iter().any(|entry| entry.starts_with("delete:")),
            "the declared rows are not this driver's to retire: {entries:?}"
        );
        assert_eq!(
            entries.iter().filter(|entry| entry.as_str() == "effect:tpm").count(),
            1,
            "the pass still runs its typed effect: {entries:?}"
        );
    }

    // -- finalize: owned children retire before the provider stage (F3) ------

    #[tokio::test]
    async fn finalize_finalizes_owned_children_before_the_provider_stage() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![owned_row("dev", "Process", "swtpm")],
        );
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;

        // A live owned child: the pass requeues and the Provider teardown
        // stage does not run.
        let failure = driver.finalize(&mut fixture.ctx).await.expect_err("owned child still live");
        assert_eq!(failure.class(), FailureClass::Retryable);
        let entries = log.lock().clone();
        assert!(
            entries.iter().any(|entry| entry == "delete:Process/swtpm"),
            "the owned child is nudged through its own finalize-before-delete pass: {entries:?}"
        );
        assert!(
            !entries.iter().any(|entry| entry.starts_with("finalize:")),
            "the Provider teardown stage has not run: {entries:?}"
        );

        // The manager removed the retired child row: the same pass converges
        // without any Provider stage.
        driver.finalize(&mut fixture.ctx).await.expect("converged once the child retired");
        assert!(
            !log.lock().iter().any(|entry| entry.starts_with("finalize:")),
            "finalize runs no Provider effect"
        );
    }

    /// Delete runs the Provider teardown stage first, then retires the owned
    /// children.
    #[tokio::test]
    async fn delete_runs_provider_teardown_then_retires_children() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![owned_row("dev", "Process", "swtpm")],
        );
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        driver.delete(&mut fixture.ctx).await.expect("delete converges");
        let entries = log.lock().clone();
        let finalize_at = entries.iter().position(|entry| entry == "finalize:device-tpm").expect("teardown ran");
        let delete_at = entries.iter().position(|entry| entry == "delete:Process/swtpm").expect("child retired");
        assert!(finalize_at < delete_at, "{entries:?}");
    }

    /// A Provider teardown stage that is still progressing is retryable: the
    /// actor re-enters delete (R10) instead of reporting completion.
    #[tokio::test]
    async fn delete_is_retryable_while_the_provider_stage_is_pending() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            RecordingManager::new(Arc::clone(&log)),
            Arc::new(RecordingRequeue::default()),
            log,
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Pending,
        );
        let mut driver = driver(&fixture).await;
        let failure = driver.delete(&mut fixture.ctx).await.expect_err("still progressing");
        assert_eq!(
            failure,
            d2b_resource_runtime::error::DriverFailure::retryable(
                d2b_resource_runtime::error::DriverOp::Delete
            )
        );
    }

    /// A kind that owns no manager rows (its children are broker/pidfd
    /// derived) adopts immediately (F2); the child-bearing binding kinds
    /// assert the same property through their ensure-before-effect ordering
    /// once a fully typed binding fixture is available.
    #[tokio::test]
    async fn recover_adopts_a_kind_that_realizes_no_managed_children() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut fixture = fixture(
            DEVICE_TYPE_NAME,
            "dev-row",
            device_spec(d2b_provider_device_tpm::PROVIDER_REF),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Pending,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );
    }

    /// §36 provider failure (`shared provider actor restarts` + `affected
    /// resources reconcile after provider restart`): after a restart the
    /// child-bearing shared provider kind adopts when its complete declared
    /// child set is still committed; a child row missing while the provider
    /// was down reports Missing, and the following reconcile re-commits the
    /// whole declared set before its typed effect runs.
    #[tokio::test]
    async fn recover_adopts_the_committed_child_set_and_reconcile_recommits_a_missing_child() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let spec = d2b_contracts_resource::v3::network::NetworkSpec::minimal(
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse("10.20.0.0/24").expect("lan"),
            d2b_contracts_resource::v3::network::Ipv4Cidr::parse("192.0.2.0/30").expect("uplink"),
            d2b_contracts_resource::v3::execution_policy::BoundedToken::parse("net-vm-base")
                .expect("token"),
        )
        .expect("network spec");
        let mut value = serde_json::to_value(&spec).expect("network spec json");
        value
            .as_object_mut()
            .expect("spec object")
            .insert("providerRef".to_owned(), json!("Provider/network-local"));
        // The deterministic child set the row declares (F1).
        let uid = super::resource_uid(&[0x42; 16]).expect("test row uid");
        let vm = d2b_provider_network_local::ifname::derive_network_child_name(&uid, "vm");
        let agent = d2b_provider_network_local::ifname::derive_network_child_name(&uid, "agent");
        let manager = RecordingManager::with_owned(
            Arc::clone(&log),
            vec![
                owned_row("dev", "Volume", "net-config"),
                owned_row("dev", "Guest", &vm),
                owned_row("dev", "Process", &agent),
            ],
        );
        let mut fixture = fixture(
            NETWORK_TYPE_NAME,
            "net-main",
            value,
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;

        // Restart with the complete child set still committed: adopt.
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted
        );

        // One declared child row retired while the provider was down: the
        // restart reports Missing instead of pretending it converged.
        manager.owned.lock().retain(|row| row.key.name != vm);
        assert_eq!(
            driver.recover(&mut fixture.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );
        assert!(
            !log.lock().iter().any(|entry| entry.starts_with("effect:")),
            "recovery adoption runs no provider effect"
        );

        // The next reconcile re-commits every declared child before the
        // effect that consumes them runs.
        driver.reconcile(&mut fixture.ctx).await.expect("reconcile");
        let entries = log.lock().clone();
        let effect_at =
            entries.iter().position(|entry| entry == "effect:network").expect("effect ran");
        for id in [
            "ensure:Volume/net-config".to_owned(),
            format!("ensure:Guest/{vm}"),
            format!("ensure:Process/{agent}"),
        ] {
            let at = entries
                .iter()
                .position(|entry| entry == &id)
                .expect("the declared child set is re-committed");
            assert!(at < effect_at, "the child set re-commits before the effect: {entries:?}");
        }
    }

    /// KTD13: this driver's only mutation surface is the manager child
    /// endpoint - reconcile of a Process-owning kind records manager ensures
    /// and the typed effect and nothing else (no spawn/launch surface exists
    /// on the port or the context).
    #[tokio::test]
    async fn reconcile_has_no_spawn_surface_beyond_the_manager_child_endpoint() {
        let log: Log = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut fixture = fixture(
            d2b_provider_device_usbip::USB_BINDING_RESOURCE_TYPE,
            "binding",
            json!({
                "name": "binding",
                "providerRef": d2b_provider_device_usbip::PROVIDER_REF,
            }),
            Arc::clone(&manager),
            Arc::new(RecordingRequeue::default()),
            Arc::clone(&log),
            SharedProviderEffectPhase::Ready,
            SharedProviderFinalize::Complete,
        );
        let mut driver = driver(&fixture).await;
        let _ = driver.reconcile(&mut fixture.ctx).await;
        let entries = log.lock().clone();
        assert!(
            entries.iter().all(|entry| entry.starts_with("ensure:") || entry.starts_with("effect:") || entry.starts_with("delete:")),
            "{entries:?}"
        );
    }
}
