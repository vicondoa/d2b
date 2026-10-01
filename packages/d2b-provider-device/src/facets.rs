//! The declared facets the provider-owned Device effects service reaches
//! daemon state through (U12 device step).
//!
//! The Device family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the reconcile and finalize orchestration over the
//! daemon's admission, child rows, and readiness state, and the per-Device
//! controller caches - crosses the provider boundary as a declared facet
//! rather than as a daemon handle: every facet here is a type the provider
//! crate declares, an implementation of it is supplied by the daemon host
//! through the composition root (never derived from caller input), and the
//! family crate holds no daemon state type.

use std::sync::Arc;
use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

use crate::binding::{DeviceBindingEvidence, DeviceInventory};
use crate::driver::{DeviceComponent, DeviceResourceState};

/// The daemon-supplied facet set the provider-owned Device effects are built
/// from (U12 device step).
///
/// The composition root supplies the object; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct DeviceEffectFacets {
    /// The daemon's Device runtime: the orchestration the driver effects
    /// delegate to, over the daemon's own admission, child rows, and
    /// readiness state.
    pub runtime: Arc<dyn DeviceRuntime>,
    /// The trusted host inventory the Device source admits its typed
    /// `DeviceBinding` relationships against.
    pub inventory: Arc<dyn DeviceInventorySource>,
    /// The graph-authority evidence the serving half re-admits a committed
    /// `DeviceBinding` row against and decides its presence from.
    pub authority: Arc<dyn DeviceBindingAuthoritySource>,
}

/// The graph-authority evidence one committed `DeviceBinding` row's presence
/// is decided from.
///
/// A serving pass may not assume a relationship it did not itself admit. The
/// committed row carries the source's accepted decision but not the
/// authorization that admitted it, the dependency fence it was fenced
/// against, or the lifecycle observed for it since - and
/// [`crate::binding::admit_device_request`] refuses a request without the
/// first two, while [`crate::binding::decide_presence`] reads the third. So
/// the evidence crosses as a declared facet: an implementation is supplied by
/// the composition root over the authority journal and the graph authority's
/// own verdict, never derived from the row being served. A plane with no
/// authority evidence behind it holds the refusal, and every relationship
/// then reports degraded rather than delivered.
#[async_trait]
pub trait DeviceBindingAuthoritySource: Send + Sync + 'static {
    /// The evidence the authority journal holds for one committed
    /// `DeviceBinding` row.
    ///
    /// `request` is the binding row's OWN effect request - its key, uid,
    /// generation, and committed spec - so the row being served is the only
    /// subject this answer may be about.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::Unavailable`] when the authority
    /// journal cannot be read for the row, and
    /// [`SharedProviderEffectError::InvalidResource`] when the row does not
    /// decode as a `DeviceBinding` or the journal holds no evidence for it.
    async fn binding_evidence(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceBindingEvidence, SharedProviderEffectError>;
}

/// The trusted physical inventory one `Device` row resolves to.
///
/// The Device source admits an exact named capability only when the trusted
/// inventory resolved that name to a physical authority and the host still
/// backs it. This facet is the only place that resolution crosses into the
/// family crate: an implementation is supplied by the composition root over
/// the verified host device-node matrix, never derived from a caller's
/// device-node path, serial, or template name. A row whose inventory cannot
/// be resolved admits nothing, which is the fail-closed answer rather than a
/// claim the source could not prove.
#[async_trait]
pub trait DeviceInventorySource: Send + Sync + 'static {
    /// Resolve the named capabilities one Device row declares into opaque
    /// physical authorities, with the presence observed for each.
    ///
    /// The committed row is the only input: the declared `DeviceSpec` names
    /// the inventory selector, and the resolved physical authority keys and
    /// their presence come from the verified host device-node matrix.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::InvalidResource`] when the
    /// committed spec does not decode as a Device spec, when the declared
    /// selector names a bus class with no trusted inventory, or when the host
    /// device-node matrix cannot be read for it.
    async fn device_inventory(
        &self,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<DeviceInventory, SharedProviderEffectError>;
}


/// The daemon-hosted Device runtime one zone's effects run over (U12 device
/// step).
///
/// The daemon implements this trait in its composition root (the same
/// shared-provider effects adapter that serves the other shared families),
/// supplying the reconcile/finalize orchestration over the daemon's own
/// admission, child rows, and readiness state. The family crate's effects
/// service delegates the driver seam to it; the per-component drives (the
/// TPM controller over the TPM crate's port, the USBIP and security-key
/// service fences, the authority-fenced GPU lifecycle) run inside the
/// runtime over the component crates' own ports, built from those crates'
/// declared facets.
#[async_trait]
pub trait DeviceRuntime: Send + Sync + 'static {
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