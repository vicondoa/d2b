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